// Слой COM: Core Audio, подписки на события Windows и слежение за процессами.
// Живёт только в потоке звука (MTA, см. mod.rs) и наружу отдаёт чистую модель из model.rs.
//
// Обратные вызовы Windows ничего не читают сами: шлют метку в канал потока, и тот перечитывает
// затронутое. Так требуют правила Core Audio — в обратном вызове нельзя делать долгих вызовов.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, E_INVALIDARG, HANDLE, INVALID_HANDLE_VALUE, PROPERTYKEY, S_OK};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl, IAudioMeterInformation,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, AudioSessionDisconnectReason, AudioSessionState, AudioSessionStateExpired,
    DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionEvents, IAudioSessionEvents_Impl, IAudioSessionManager2, IAudioSessionNotification,
    IAudioSessionNotification_Impl, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
    ISimpleAudioVolume, MMDeviceEnumerator, eCapture, eConsole, eRender,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    QueryFullProcessImageNameW, RegisterWaitForSingleObject, UnregisterWaitEx, WT_EXECUTEONLYONCE,
};
use windows::core::{GUID, Interface, PCWSTR, PWSTR, Ref, implement, w};

use super::AudioError;
use super::Msg;
use super::model::{self, Device, Flow, RawSession, Snapshot, Target};

/// Сколько подписок на события Windows сейчас держит ядро. После выхода должно быть 0.
static SUBS: AtomicIsize = AtomicIsize::new(0);

pub fn live_subscriptions() -> isize {
    SUBS.load(Ordering::SeqCst)
}

const STILL_ACTIVE: u32 = 259;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
    s
}

fn device_id(d: &IMMDevice) -> windows::core::Result<String> {
    Ok(take_pwstr(unsafe { d.GetId()? }))
}

fn device_name(d: &IMMDevice) -> windows::core::Result<String> {
    unsafe {
        let store = d.OpenPropertyStore(STGM_READ)?;
        let value = store.GetValue(&PKEY_Device_FriendlyName)?;
        Ok(take_pwstr(PropVariantToStringAlloc(&value)?))
    }
}

fn win(e: windows::core::Error) -> AudioError {
    AudioError::Windows(e.message())
}

// ---------- обратные вызовы ----------

#[implement(IMMNotificationClient)]
struct DeviceClient {
    tx: Sender<Msg>,
}

impl IMMNotificationClient_Impl for DeviceClient_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::Devices);
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::Devices);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::Devices);
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, _: EDataFlow, _: ERole, _: &PCWSTR) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::Devices);
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, key: &PROPERTYKEY) -> windows::core::Result<()> {
        // Свойства меняются часто и по мелочи — нам важно только имя.
        if *key == PKEY_Device_FriendlyName {
            let _ = self.tx.send(Msg::Devices);
        }
        Ok(())
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeChanged {
    tx: Sender<Msg>,
    device: String,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeChanged_Impl {
    fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::Device(self.device.clone()));
        Ok(())
    }
}

#[implement(IAudioSessionNotification)]
struct NewSession {
    tx: Sender<Msg>,
    device: String,
}

impl IAudioSessionNotification_Impl for NewSession_Impl {
    fn OnSessionCreated(&self, session: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        // Саму сессию отдаём потоку звука: перебирать список заново нельзя (см. `enumerate_sessions`).
        if let Ok(c) = session.ok() {
            let _ = self.tx.send(Msg::NewSession(self.device.clone(), NewControl(c.clone())));
        }
        Ok(())
    }
}

/// Новая сессия из уведомления Windows по дороге в поток звука. Оба потока — MTA, а объекты
/// Core Audio в MTA можно звать из любого его потока, поэтому переносить указатель можно.
pub struct NewControl(IAudioSessionControl);

unsafe impl Send for NewControl {}

#[implement(IAudioSessionEvents)]
struct SessionEvents {
    tx: Sender<Msg>,
    device: String,
}

impl SessionEvents_Impl {
    fn mark(&self) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::Device(self.device.clone()));
        Ok(())
    }
}

impl IAudioSessionEvents_Impl for SessionEvents_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        self.mark()
    }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnSimpleVolumeChanged(&self, _: f32, _: windows::core::BOOL, _: *const GUID) -> windows::core::Result<()> {
        self.mark()
    }
    fn OnChannelVolumeChanged(&self, _: u32, _: *const f32, _: u32, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnGroupingParamChanged(&self, _: *const GUID, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnStateChanged(&self, _: AudioSessionState) -> windows::core::Result<()> {
        self.mark()
    }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> windows::core::Result<()> {
        self.mark()
    }
}

// ---------- процессы ----------

/// Ожидание конца процесса: Windows сама дёрнет `on_exit` в своём пуле потоков.
struct ProcWatch {
    process: HANDLE,
    wait: HANDLE,
    ctx: *mut Sender<Msg>,
}

unsafe extern "system" fn on_exit(ctx: *mut c_void, _timed_out: bool) {
    let tx = unsafe { &*(ctx as *const Sender<Msg>) };
    let _ = tx.send(Msg::Sessions);
}

impl Drop for ProcWatch {
    fn drop(&mut self) {
        unsafe {
            // INVALID_HANDLE_VALUE — дождаться, пока обратный вызов точно закончится, и только потом
            // освободить его контекст.
            let _ = UnregisterWaitEx(self.wait, Some(INVALID_HANDLE_VALUE));
            drop(Box::from_raw(self.ctx));
            let _ = CloseHandle(self.process);
        }
        SUBS.fetch_sub(1, Ordering::SeqCst);
    }
}

struct ProcInfo {
    path: Option<String>,
    alive: bool,
    watch: Option<ProcWatch>,
}

impl ProcInfo {
    /// Жив ли процесс сейчас. Для процессов с ожиданием — спросить Windows: `on_exit` только будит
    /// поток, а отметку о смерти не ставит.
    fn alive(&self) -> bool {
        match &self.watch {
            Some(w) => {
                let mut code = 0u32;
                unsafe { GetExitCodeProcess(w.process, &mut code) }.is_err() || code == STILL_ACTIVE
            }
            None => self.alive,
        }
    }
}

fn open_process(pid: u32, tx: &Sender<Msg>) -> ProcInfo {
    let unknown = ProcInfo { path: None, alive: true, watch: None };
    let process = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(h) => h,
        // Такого процесса нет. Остальные отказы (нет доступа) — процесс жив, но закрыт.
        Err(e) if e.code() == E_INVALIDARG => return ProcInfo { alive: false, ..unknown },
        Err(_) => return unknown,
    };
    let mut code = 0u32;
    if unsafe { GetExitCodeProcess(process, &mut code) }.is_ok() && code != STILL_ACTIVE {
        unsafe {
            let _ = CloseHandle(process);
        }
        return ProcInfo { alive: false, ..unknown };
    }
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let path = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }
        .ok()
        .map(|_| String::from_utf16_lossy(&buf[..len as usize]));
    let ctx = Box::into_raw(Box::new(tx.clone()));
    let mut wait = HANDLE::default();
    let registered = unsafe {
        RegisterWaitForSingleObject(
            &mut wait,
            process,
            Some(on_exit),
            Some(ctx as *const c_void),
            u32::MAX,
            WT_EXECUTEONLYONCE,
        )
    };
    let watch = if registered.is_ok() {
        SUBS.fetch_add(1, Ordering::SeqCst);
        Some(ProcWatch { process, wait, ctx })
    } else {
        unsafe {
            drop(Box::from_raw(ctx));
            let _ = CloseHandle(process);
        }
        None
    };
    ProcInfo { path, alive: true, watch }
}

/// Описание файла из его ресурсов версии («Google Chrome» вместо «chrome»).
fn file_description(path: &str) -> Option<String> {
    let w = wide(path);
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, size, buf.as_mut_ptr() as *mut c_void).ok()?;
        let mut p: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(buf.as_ptr() as *const c_void, w!("\\VarFileInfo\\Translation"), &mut p, &mut len).as_bool()
            || len < 4
        {
            return None;
        }
        let lang = *(p as *const u16);
        let cp = *(p as *const u16).add(1);
        let sub = wide(&format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\FileDescription"));
        if !VerQueryValueW(buf.as_ptr() as *const c_void, PCWSTR(sub.as_ptr()), &mut p, &mut len).as_bool() || len == 0
        {
            return None;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize));
        let s = s.trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

// ---------- сессии и устройства ----------

struct SessionHandle {
    id: String,
    ctrl: IAudioSessionControl2,
    simple: ISimpleAudioVolume,
    meter: Option<IAudioMeterInformation>,
    events: IAudioSessionEvents,
    raw: RawSession,
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        if unsafe { self.ctrl.UnregisterAudioSessionNotification(&self.events) }.is_ok() {
            SUBS.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

struct DeviceHandle {
    info: Device,
    volume: IAudioEndpointVolume,
    volume_cb: IAudioEndpointVolumeCallback,
    meter: Option<IAudioMeterInformation>,
    manager: Option<IAudioSessionManager2>,
    new_cb: IAudioSessionNotification,
    sessions: Vec<SessionHandle>,
}

impl Drop for DeviceHandle {
    fn drop(&mut self) {
        unsafe {
            if self.volume.UnregisterControlChangeNotify(&self.volume_cb).is_ok() {
                SUBS.fetch_sub(1, Ordering::SeqCst);
            }
            if let Some(m) = &self.manager
                && m.UnregisterSessionNotification(&self.new_cb).is_ok()
            {
                SUBS.fetch_sub(1, Ordering::SeqCst);
            }
        }
        // Сессии снимают свои подписки сами, когда поле `sessions` удаляется следом.
    }
}

impl DeviceHandle {
    fn open(dev: &IMMDevice, id: String, flow: Flow, tx: &Sender<Msg>) -> windows::core::Result<Self> {
        unsafe {
            let volume: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None)?;
            let volume_cb: IAudioEndpointVolumeCallback = VolumeChanged { tx: tx.clone(), device: id.clone() }.into();
            if volume.RegisterControlChangeNotify(&volume_cb).is_ok() {
                SUBS.fetch_add(1, Ordering::SeqCst);
            }
            let meter = dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok();
            let new_cb: IAudioSessionNotification = NewSession { tx: tx.clone(), device: id.clone() }.into();
            let manager = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None).ok();
            if let Some(m) = &manager
                && m.RegisterSessionNotification(&new_cb).is_ok()
            {
                SUBS.fetch_add(1, Ordering::SeqCst);
            }
            let info = Device {
                id,
                name: String::new(),
                flow,
                volume: 0.0,
                muted: false,
                is_default: false,
                apps: Vec::new(),
            };
            Ok(DeviceHandle { info, volume, volume_cb, meter, manager, new_cb, sessions: Vec::new() })
        }
    }

    fn read_volume(&mut self) {
        unsafe {
            if let Ok(v) = self.volume.GetMasterVolumeLevelScalar() {
                self.info.volume = v;
            }
            if let Ok(m) = self.volume.GetMute() {
                self.info.muted = m.as_bool();
            }
        }
    }
}

/// Имя, громкость и «без звука» сессии.
fn read_session(s: &mut SessionHandle, descriptions: &mut HashMap<String, Option<String>>) {
    // Имя: из сессии (если это не ссылка на ресурс «@…»), иначе описание файла.
    let display = unsafe { s.ctrl.GetDisplayName() }.map(take_pwstr).unwrap_or_default();
    s.raw.name = if !display.is_empty() && !display.starts_with('@') {
        display
    } else if let Some(p) = &s.raw.path {
        descriptions.entry(p.clone()).or_insert_with(|| file_description(p)).clone().unwrap_or_default()
    } else {
        String::new()
    };
    unsafe {
        if let Ok(v) = s.simple.GetMasterVolume() {
            s.raw.volume = v;
        }
        if let Ok(m) = s.simple.GetMute() {
            s.raw.muted = m.as_bool();
        }
    }
}

/// Всё, что ядро знает о звуке, плюс подписки на изменения.
pub struct Engine {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
    tx: Sender<Msg>,
    devices: Vec<DeviceHandle>,
    procs: HashMap<u32, ProcInfo>,
    descriptions: HashMap<String, Option<String>>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            if self.enumerator.UnregisterEndpointNotificationCallback(&self.client).is_ok() {
                SUBS.fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.devices.clear();
        self.procs.clear();
    }
}

impl Engine {
    pub fn new(tx: Sender<Msg>) -> windows::core::Result<Self> {
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let client: IMMNotificationClient = DeviceClient { tx: tx.clone() }.into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client)? };
        SUBS.fetch_add(1, Ordering::SeqCst);
        let mut e =
            Engine { enumerator, client, tx, devices: Vec::new(), procs: HashMap::new(), descriptions: HashMap::new() };
        e.rescan_devices();
        Ok(e)
    }

    /// Перечитать список устройств; у прежних устройств подписки остаются.
    pub fn rescan_devices(&mut self) {
        let mut old = std::mem::take(&mut self.devices);
        let mut opened = Vec::new();
        for (edf, flow) in [(eRender, Flow::Render), (eCapture, Flow::Capture)] {
            let default_id =
                unsafe { self.enumerator.GetDefaultAudioEndpoint(edf, eConsole) }.ok().and_then(|d| device_id(&d).ok());
            let Ok(list) = (unsafe { self.enumerator.EnumAudioEndpoints(edf, DEVICE_STATE_ACTIVE) }) else {
                continue;
            };
            let count = unsafe { list.GetCount() }.unwrap_or(0);
            for i in 0..count {
                let Ok(dev) = (unsafe { list.Item(i) }) else { continue };
                let Ok(id) = device_id(&dev) else { continue };
                let mut handle = match old.iter().position(|h| h.info.id == id) {
                    Some(p) => old.swap_remove(p),
                    None => match DeviceHandle::open(&dev, id.clone(), flow, &self.tx) {
                        Ok(h) => {
                            opened.push(self.devices.len());
                            h
                        }
                        Err(_) => continue,
                    },
                };
                handle.info.name = device_name(&dev).unwrap_or_default();
                handle.info.is_default = default_id.as_deref() == Some(id.as_str());
                handle.read_volume();
                self.devices.push(handle);
            }
        }
        drop(old);
        for i in 0..self.devices.len() {
            if opened.contains(&i) {
                self.enumerate_sessions(i);
            } else {
                self.refresh_sessions(i);
            }
        }
        self.forget_dead_procs();
    }

    /// Перечитать одно устройство: громкость и его программы.
    pub fn rescan_device(&mut self, id: &str) {
        if let Some(i) = self.devices.iter().position(|h| h.info.id == id) {
            self.devices[i].read_volume();
            self.refresh_sessions(i);
            self.forget_dead_procs();
        }
    }

    /// Перечитать программы всех устройств (например, закрылся процесс).
    pub fn rescan_sessions(&mut self) {
        for i in 0..self.devices.len() {
            self.refresh_sessions(i);
        }
        self.forget_dead_procs();
    }

    /// Сессия из уведомления Windows о новой сессии.
    pub fn add_session(&mut self, device: &str, ctrl: NewControl) {
        if let Some(i) = self.devices.iter().position(|h| h.info.id == device) {
            self.take_session(i, ctrl.0);
        }
    }

    fn forget_dead_procs(&mut self) {
        let mut seen: Vec<u32> = Vec::new();
        for h in &self.devices {
            seen.extend(h.sessions.iter().map(|s| s.raw.pid));
        }
        self.procs.retain(|pid, _| seen.contains(pid));
    }

    // Список сессий перебирается только здесь, один раз при открытии устройства. Каждый перебор,
    // пока у нас на руках объекты сессий, оставляет в памяти процесса ~450 байт на каждую из них
    // (замер 22.10.2026: ~5 КБ на проход по всем устройствам), и Windows их не возвращает. Дальше
    // новые сессии приходят в `OnSessionCreated`, а прежние перечитываются через свои объекты.
    fn enumerate_sessions(&mut self, i: usize) {
        let Some(manager) = &self.devices[i].manager else { return };
        let Ok(list) = (unsafe { manager.GetSessionEnumerator() }) else { return };
        let count = unsafe { list.GetCount() }.unwrap_or(0);
        for k in 0..count {
            if let Ok(ctrl) = unsafe { list.GetSession(k) } {
                self.take_session(i, ctrl);
            }
        }
    }

    /// Завести сессию, если её ещё нет, и прочитать её.
    fn take_session(&mut self, i: usize, ctrl: IAudioSessionControl) {
        let Engine { devices, procs, descriptions, tx, .. } = self;
        let h = &mut devices[i];
        if unsafe { ctrl.GetState() }.map(|s| s == AudioSessionStateExpired).unwrap_or(true) {
            return;
        }
        let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else { return };
        let Ok(id) = (unsafe { ctrl2.GetSessionInstanceIdentifier() }).map(take_pwstr) else { return };
        if h.sessions.iter().any(|s| s.id == id) {
            return;
        }
        let system = unsafe { ctrl2.IsSystemSoundsSession() } == S_OK;
        let pid = unsafe { ctrl2.GetProcessId() }.unwrap_or(0);
        let proc = procs.entry(pid).or_insert_with(|| {
            if system { ProcInfo { path: None, alive: true, watch: None } } else { open_process(pid, tx) }
        });
        if !proc.alive() {
            return;
        }
        let Ok(simple) = ctrl.cast::<ISimpleAudioVolume>() else { return };
        let events: IAudioSessionEvents = SessionEvents { tx: tx.clone(), device: h.info.id.clone() }.into();
        if unsafe { ctrl.RegisterAudioSessionNotification(&events) }.is_ok() {
            SUBS.fetch_add(1, Ordering::SeqCst);
        }
        let meter = ctrl.cast::<IAudioMeterInformation>().ok();
        let raw = RawSession { pid, path: proc.path.clone(), name: String::new(), volume: 1.0, muted: false, system };
        let mut s = SessionHandle { id, ctrl: ctrl2, simple, meter, events, raw };
        read_session(&mut s, descriptions);
        h.sessions.push(s);
    }

    /// Перечитать сессии устройства через их объекты; ушедшие (истёкшие, процесс закрыт) — убрать.
    fn refresh_sessions(&mut self, i: usize) {
        let Engine { devices, procs, descriptions, .. } = self;
        let h = &mut devices[i];
        h.sessions.retain(|s| {
            let expired = unsafe { s.ctrl.GetState() }.map(|st| st == AudioSessionStateExpired).unwrap_or(true);
            let alive = s.raw.system || procs.get(&s.raw.pid).is_none_or(|p| p.alive());
            !expired && alive
        });
        for s in &mut h.sessions {
            read_session(s, descriptions);
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let devices = self
            .devices
            .iter()
            .map(|h| {
                let raw: Vec<RawSession> = h.sessions.iter().map(|s| s.raw.clone()).collect();
                Device { apps: model::group(&h.info.id, &raw), ..h.info.clone() }
            })
            .collect();
        Snapshot { devices }
    }

    fn device(&self, id: &str) -> Result<&DeviceHandle, AudioError> {
        self.devices.iter().find(|h| h.info.id == id).ok_or(AudioError::NotFound)
    }

    fn app_sessions<'a>(&'a self, key: &model::AppKey) -> Result<Vec<&'a SessionHandle>, AudioError> {
        let h = self.device(&key.device)?;
        let list: Vec<&SessionHandle> = h.sessions.iter().filter(|s| model::exe_key(&s.raw) == key.exe).collect();
        if list.is_empty() { Err(AudioError::NotFound) } else { Ok(list) }
    }

    pub fn set_volume(&self, target: &Target, level: f32) -> Result<(), AudioError> {
        let level = level.clamp(0.0, 1.0);
        match target {
            Target::Device(id) => unsafe {
                self.device(id)?.volume.SetMasterVolumeLevelScalar(level, std::ptr::null()).map_err(win)
            },
            Target::App(key) => {
                for s in self.app_sessions(key)? {
                    unsafe { s.simple.SetMasterVolume(level, std::ptr::null()) }.map_err(win)?;
                }
                Ok(())
            }
        }
    }

    /// Сдвинуть громкость устройства вывода по умолчанию. Текущую громкость читаем у Windows, а не из
    /// снимка: при быстрой прокрутке колеса снимок ещё не успел обновиться.
    pub fn adjust_default(&self, delta: f32) -> Result<f32, AudioError> {
        let h = self
            .devices
            .iter()
            .find(|h| h.info.flow == Flow::Render && h.info.is_default)
            .ok_or(AudioError::NotFound)?;
        unsafe {
            let now = h.volume.GetMasterVolumeLevelScalar().map_err(win)?;
            let level = model::step(now, delta);
            h.volume.SetMasterVolumeLevelScalar(level, std::ptr::null()).map_err(win)?;
            Ok(level)
        }
    }

    pub fn set_muted(&self, target: &Target, muted: bool) -> Result<(), AudioError> {
        match target {
            Target::Device(id) => unsafe { self.device(id)?.volume.SetMute(muted, std::ptr::null()).map_err(win) },
            Target::App(key) => {
                for s in self.app_sessions(key)? {
                    unsafe { s.simple.SetMute(muted, std::ptr::null()) }.map_err(win)?;
                }
                Ok(())
            }
        }
    }

    /// Пиковые уровни 0..1 всех устройств и программ — читаются только здесь, по запросу.
    pub fn peaks(&self) -> Vec<(Target, f32)> {
        let mut out = Vec::new();
        for h in &self.devices {
            let peak = h.meter.as_ref().and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0);
            out.push((Target::Device(h.info.id.clone()), peak));
            let mut apps: Vec<(model::AppKey, f32)> = Vec::new();
            for s in &h.sessions {
                let p = s.meter.as_ref().and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0);
                let key = model::AppKey { device: h.info.id.clone(), exe: model::exe_key(&s.raw) };
                match apps.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, v)) => *v = v.max(p),
                    None => apps.push((key, p)),
                }
            }
            out.extend(apps.into_iter().map(|(k, p)| (Target::App(k), p)));
        }
        out
    }
}
