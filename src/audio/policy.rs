// Недокументированные API Windows — только здесь. Сбой отключает свою функцию, а не программу.
//
// - `IPolicyConfig` (COM, CLSID 870AF99C-…): сменить устройство по умолчанию — так делает панель «Звук».
// - `Windows.Media.Internal.AudioPolicyConfig` (WinRT-фабрика): закрепить устройство за процессом — так
//   делают «Параметры → Громкость приложений». IID фабрики разный у сборок Windows: пробуем оба известных.

#![allow(non_snake_case)]

use std::ffi::c_void;

use windows::Win32::Media::Audio::{EDataFlow, ERole, eCapture, eCommunications, eConsole, eMultimedia, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::WinRT::RoGetActivationFactory;
use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, PCWSTR, interface};

use super::model::Flow;

#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    // Первые десять методов не нужны, но стоят на своих местах в таблице.
    fn Unused1(&self) -> HRESULT;
    fn Unused2(&self) -> HRESULT;
    fn Unused3(&self) -> HRESULT;
    fn Unused4(&self) -> HRESULT;
    fn Unused5(&self) -> HRESULT;
    fn Unused6(&self) -> HRESULT;
    fn Unused7(&self) -> HRESULT;
    fn Unused8(&self) -> HRESULT;
    fn GetPropertyValue(&self) -> HRESULT;
    fn SetPropertyValue(&self) -> HRESULT;
    fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
}

const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// Таблица `AudioPolicyConfig` одна и та же, меняется только IID — объявляем её дважды. Это интерфейс
/// WinRT (`IInspectable`), но макросу хватает `IUnknown` и трёх методов `IInspectable` в начале таблицы.
macro_rules! policy_factory {
    ($name:ident, $iid:literal) => {
        #[interface($iid)]
        unsafe trait $name: IUnknown {
            fn GetIids(&self) -> HRESULT;
            fn GetRuntimeClassName(&self) -> HRESULT;
            fn GetTrustLevel(&self) -> HRESULT;
            fn add_CtxVolumeChange(&self) -> HRESULT;
            fn remove_CtxVolumeChanged(&self) -> HRESULT;
            fn add_RingerVibrateStateChanged(&self) -> HRESULT;
            fn remove_RingerVibrateStateChange(&self) -> HRESULT;
            fn SetVolumeGroupGainForId(&self) -> HRESULT;
            fn GetVolumeGroupGainForId(&self) -> HRESULT;
            fn GetActiveVolumeGroupForEndpointId(&self) -> HRESULT;
            fn GetVolumeGroupsForEndpoint(&self) -> HRESULT;
            fn GetCurrentVolumeContext(&self) -> HRESULT;
            fn SetVolumeGroupMuteForId(&self) -> HRESULT;
            fn GetVolumeGroupMuteForId(&self) -> HRESULT;
            fn SetRingerVibrateState(&self) -> HRESULT;
            fn GetRingerVibrateState(&self) -> HRESULT;
            fn SetPreferredChatApplication(&self) -> HRESULT;
            fn ResetPreferredChatApplication(&self) -> HRESULT;
            fn GetPreferredChatApplication(&self) -> HRESULT;
            fn GetCurrentChatApplications(&self) -> HRESULT;
            fn add_ChatContextChanged(&self) -> HRESULT;
            fn remove_ChatContextChanged(&self) -> HRESULT;
            fn SetPersistedDefaultAudioEndpoint(
                &self,
                pid: u32,
                flow: EDataFlow,
                role: ERole,
                id: *mut c_void,
            ) -> HRESULT;
            fn GetPersistedDefaultAudioEndpoint(
                &self,
                pid: u32,
                flow: EDataFlow,
                role: ERole,
                id: *mut *mut c_void,
            ) -> HRESULT;
        }
    };
}

policy_factory!(IAudioPolicyConfigNew, "ab3d4648-e242-459f-b02f-541c70306324");
policy_factory!(IAudioPolicyConfigOld, "2a59116d-6c4f-45e0-a74f-707e3fef9258");

enum Factory {
    New(IAudioPolicyConfigNew),
    Old(IAudioPolicyConfigOld),
}

const MMDEVAPI_TOKEN: &str = r"\\?\SWD#MMDEVAPI#";
const RENDER_SUFFIX: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const CAPTURE_SUFFIX: &str = "#{2eef81be-33fa-4800-9670-1cd474972c3f}";

/// Id устройства Core Audio → id для `AudioPolicyConfig`.
pub fn pack_id(id: &str, flow: Flow) -> String {
    let suffix = if flow == Flow::Render { RENDER_SUFFIX } else { CAPTURE_SUFFIX };
    format!("{MMDEVAPI_TOKEN}{id}{suffix}")
}

/// Обратно; пусто — не закреплено.
pub fn unpack_id(packed: &str) -> Option<String> {
    let s = packed.strip_prefix(MMDEVAPI_TOKEN).unwrap_or(packed);
    let s = s.strip_suffix(RENDER_SUFFIX).or_else(|| s.strip_suffix(CAPTURE_SUFFIX)).unwrap_or(s);
    (!s.is_empty()).then(|| s.to_string())
}

fn data_flow(flow: Flow) -> EDataFlow {
    if flow == Flow::Render { eRender } else { eCapture }
}

/// Какие роли назначить устройству.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Roles {
    /// По умолчанию: console, multimedia и communications — как панель «Звук».
    All,
    /// Только устройство связи.
    Communications,
}

pub struct Policy {
    config: Option<IPolicyConfig>,
    factory: Option<Factory>,
}

impl Policy {
    /// Создать в потоке звука (COM уже готов). Чего не вышло — то и недоступно.
    pub fn new() -> Policy {
        let config = unsafe { CoCreateInstance::<_, IPolicyConfig>(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL) }.ok();
        let class = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
        let factory = unsafe { RoGetActivationFactory::<IAudioPolicyConfigNew>(&class) }
            .map(Factory::New)
            .or_else(|_| unsafe { RoGetActivationFactory::<IAudioPolicyConfigOld>(&class) }.map(Factory::Old))
            .ok();
        Policy { config, factory }
    }

    pub fn can_route(&self) -> bool {
        self.factory.is_some()
    }

    pub fn set_default(&self, id: &str, roles: Roles) -> windows::core::Result<()> {
        let config =
            self.config.as_ref().ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_NOTIMPL))?;
        let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        let list: &[ERole] = match roles {
            Roles::All => &[eConsole, eMultimedia, eCommunications],
            Roles::Communications => &[eCommunications],
        };
        for role in list {
            unsafe { config.SetDefaultEndpoint(PCWSTR(wide.as_ptr()), *role) }.ok()?;
        }
        Ok(())
    }

    /// Закрепить устройство за процессом; `None` — «как в системе» (снять закрепление).
    pub fn set_route(&self, pid: u32, flow: Flow, device: Option<&str>) -> windows::core::Result<()> {
        let factory =
            self.factory.as_ref().ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_NOTIMPL))?;
        let hs = device.map(|d| HSTRING::from(pack_id(d, flow))).unwrap_or_default();
        // HSTRING — прозрачная обёртка над дескриптором строки; пустая — нулевой дескриптор.
        let raw: *mut c_void = unsafe { std::mem::transmute_copy(&hs) };
        for role in [eMultimedia, eConsole] {
            let hr = unsafe {
                match factory {
                    Factory::New(f) => f.SetPersistedDefaultAudioEndpoint(pid, data_flow(flow), role, raw),
                    Factory::Old(f) => f.SetPersistedDefaultAudioEndpoint(pid, data_flow(flow), role, raw),
                }
            };
            hr.ok()?;
        }
        Ok(())
    }

    /// Устройство, за которым закреплён процесс (`None` — не закреплён или не узнать).
    pub fn route(&self, pid: u32, flow: Flow) -> Option<String> {
        let factory = self.factory.as_ref()?;
        let mut raw: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            match factory {
                Factory::New(f) => f.GetPersistedDefaultAudioEndpoint(pid, data_flow(flow), eMultimedia, &mut raw),
                Factory::Old(f) => f.GetPersistedDefaultAudioEndpoint(pid, data_flow(flow), eMultimedia, &mut raw),
            }
        };
        if hr.is_err() || raw.is_null() {
            return None;
        }
        // Владение строкой переходит к HSTRING — он её и освободит.
        let hs: HSTRING = unsafe { std::mem::transmute(raw) };
        unpack_id(&hs.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_round_trip() {
        let id = "{0.0.0.00000000}.{c22a0969-bd97-4e26-bdf0-d8f61b41e0cd}";
        let packed = pack_id(id, Flow::Render);
        assert_eq!(
            packed,
            r"\\?\SWD#MMDEVAPI#{0.0.0.00000000}.{c22a0969-bd97-4e26-bdf0-d8f61b41e0cd}#{e6327cad-dcec-4949-ae8a-991e976a79d2}"
        );
        assert_eq!(unpack_id(&packed).as_deref(), Some(id));
        assert!(pack_id(id, Flow::Capture).ends_with(CAPTURE_SUFFIX));
        assert_eq!(unpack_id(&pack_id(id, Flow::Capture)).as_deref(), Some(id));
        assert_eq!(unpack_id(""), None);
    }

    // Настоящий Windows, только чтение: фабрика создаётся, закрепление своего процесса читается.
    #[test]
    fn policy_reads_without_changing() {
        unsafe {
            let _ =
                windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let p = Policy::new();
        assert!(p.config.is_some(), "IPolicyConfig");
        eprintln!("routing available: {}", p.can_route());
        if p.can_route() {
            eprintln!("own route: {:?}", p.route(std::process::id(), Flow::Render));
        }
    }
}
