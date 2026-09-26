//! Read-only WMI view: instance enumeration of a local namespace through `IWbemServices::ExecQuery`
//! (`SELECT * FROM <class>`). No methods are executed, nothing is written, no event is
//! subscribed, and only local namespaces are connected – a remote namespace is refused.

use std::collections::BTreeMap;

use vbs_core::views::{ViewError, WmiObject, WmiValue, WmiView};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoInitializeSecurity,
    CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_AUTHN_LEVEL_DEFAULT,
    RPC_C_IMP_LEVEL_IMPERSONATE, SAFEARRAY,
};
use windows::Win32::System::Ole::{SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound};
use windows::Win32::System::Variant::{
    VARIANT, VT_ARRAY, VT_BOOL, VT_BSTR, VT_I1, VT_I2, VT_I4, VT_I8, VT_INT, VT_UI1, VT_UI2, VT_UI4, VT_UI8, VT_UINT,
    VariantClear,
};
use windows::Win32::System::Wmi::{
    IEnumWbemClassObject, IWbemClassObject, IWbemLocator, IWbemServices, WBEM_E_ACCESS_DENIED, WBEM_E_INVALID_CLASS,
    WBEM_E_INVALID_NAMESPACE, WBEM_E_NOT_FOUND, WBEM_FLAG_CONNECT_USE_MAX_WAIT, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_NONSYSTEM_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WbemLocator,
};
use windows::core::{BSTR, HRESULT, PCWSTR, w};

/// `RPC_C_AUTHN_WINNT` and `RPC_C_AUTHZ_NONE`.
const AUTHN_WINNT: u32 = 10;
const AUTHZ_NONE: u32 = 0;
/// How long one `Next` call may wait (the repository answers in milliseconds).
const TIMEOUT_MS: i32 = 30_000;
/// `E_ACCESSDENIED`
const E_ACCESS_DENIED: HRESULT = HRESULT(0x8007_0005_u32 as i32);

#[derive(Debug, Default, Clone, Copy)]
pub struct WinWmi;

/// COM for the current thread, released on drop when this call initialised it.
struct Com {
    owned: bool,
}

impl Com {
    fn init() -> Self {
        // SAFETY: initialises COM for this thread; paired with CoUninitialize in Drop when it succeeded.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // Default process security with impersonation, which WMI needs; a second call fails with
        // RPC_E_TOO_LATE and keeps the first setting, and a failure is not fatal because the proxy
        // blanket below sets impersonation per connection. Process-local, not a system change.
        // SAFETY: all pointer arguments are null/None as documented.
        let _ = unsafe {
            CoInitializeSecurity(
                None,
                -1,
                None,
                None,
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
                None,
            )
        };
        Com { owned: result.is_ok() }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: balances the successful CoInitializeEx of this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn view_error(error: &windows::core::Error) -> ViewError {
    let code = error.code();
    let wbem = |status: windows::Win32::System::Wmi::WBEMSTATUS| HRESULT(status.0);
    if [wbem(WBEM_E_INVALID_NAMESPACE), wbem(WBEM_E_INVALID_CLASS), wbem(WBEM_E_NOT_FOUND)].contains(&code) {
        ViewError::NotFound
    } else if code == wbem(WBEM_E_ACCESS_DENIED) || code == E_ACCESS_DENIED {
        ViewError::AccessDenied
    } else {
        ViewError::Failed(format!("WMI error {:#010x}", code.0))
    }
}

impl WmiView for WinWmi {
    fn instances(&self, namespace: &str, class: &str) -> Result<Vec<WmiObject>, ViewError> {
        // Local namespaces only (`ROOT\…`): a server name would mean a network connection.
        if !namespace.to_ascii_uppercase().starts_with("ROOT") || namespace.contains("//") || namespace.contains("\\\\")
        {
            return Err(ViewError::NetworkPath);
        }
        if !class.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ViewError::Failed("invalid class name".into()));
        }
        let _com = Com::init();
        // SAFETY: COM calls on interfaces created in this function; every returned object is
        // released when dropped, every VARIANT is cleared after use.
        unsafe {
            let locator: IWbemLocator =
                CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(|e| view_error(&e))?;
            let empty = BSTR::new();
            let services: IWbemServices = locator
                .ConnectServer(
                    &BSTR::from(namespace),
                    &empty,
                    &empty,
                    &empty,
                    WBEM_FLAG_CONNECT_USE_MAX_WAIT.0,
                    &empty,
                    None,
                )
                .map_err(|e| view_error(&e))?;
            CoSetProxyBlanket(
                &services,
                AUTHN_WINNT,
                AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
            .map_err(|e| view_error(&e))?;
            let query = BSTR::from(format!("SELECT * FROM {class}"));
            let enumerator: IEnumWbemClassObject = services
                .ExecQuery(&BSTR::from("WQL"), &query, WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None)
                .map_err(|e| view_error(&e))?;
            let mut objects = Vec::new();
            loop {
                let mut batch = [None];
                let mut returned = 0u32;
                let status = enumerator.Next(TIMEOUT_MS, &mut batch, &mut returned);
                if status.is_err() {
                    return Err(view_error(&windows::core::Error::from(status)));
                }
                let Some(object) = batch[0].take().filter(|_| returned > 0) else { break };
                objects.push(read_object(&object));
            }
            Ok(objects)
        }
    }
}

/// Relative path and all non-system properties of an instance.
unsafe fn read_object(object: &IWbemClassObject) -> WmiObject {
    let mut properties = BTreeMap::new();
    // SAFETY (for the whole function): `object` is a live instance; VARIANTs are cleared after reading.
    unsafe {
        let mut path = VARIANT::default();
        let relative = match object.Get(w!("__RELPATH"), 0, &mut path, None, None) {
            Ok(()) => value(&path),
            Err(_) => WmiValue::Null,
        };
        let _ = VariantClear(&mut path);
        if object.BeginEnumeration(WBEM_FLAG_NONSYSTEM_ONLY.0).is_ok() {
            loop {
                let mut name = BSTR::new();
                let mut data = VARIANT::default();
                let (mut kind, mut flavor) = (0i32, 0i32);
                if object.Next(0, &mut name, &mut data, &mut kind, &mut flavor).is_err() || name.is_empty() {
                    let _ = VariantClear(&mut data);
                    break;
                }
                properties.insert(name.to_string(), value(&data));
                let _ = VariantClear(&mut data);
            }
            let _ = object.EndEnumeration();
        }
        let path = match relative {
            WmiValue::Text(text) => text,
            _ => String::new(),
        };
        WmiObject { path, properties }
    }
}

/// The value of a VARIANT as WMI returns it (strings, numbers, booleans, string arrays).
unsafe fn value(variant: &VARIANT) -> WmiValue {
    // SAFETY: the union member read matches the VARIANT type tag.
    unsafe {
        let inner = &variant.Anonymous.Anonymous;
        let vt = inner.vt;
        match vt {
            VT_BSTR => WmiValue::Text(inner.Anonymous.bstrVal.to_string()),
            VT_BOOL => WmiValue::Bool(inner.Anonymous.boolVal.0 != 0),
            VT_I1 => WmiValue::Int(i64::from(inner.Anonymous.cVal)),
            VT_UI1 => WmiValue::Int(i64::from(inner.Anonymous.bVal)),
            VT_I2 => WmiValue::Int(i64::from(inner.Anonymous.iVal)),
            VT_UI2 => WmiValue::Int(i64::from(inner.Anonymous.uiVal)),
            VT_I4 | VT_INT => WmiValue::Int(i64::from(inner.Anonymous.lVal)),
            VT_UI4 | VT_UINT => WmiValue::Int(i64::from(inner.Anonymous.ulVal)),
            VT_I8 => WmiValue::Int(inner.Anonymous.llVal),
            VT_UI8 => WmiValue::Int(i64::try_from(inner.Anonymous.ullVal).unwrap_or(i64::MAX)),
            _ if vt.0 == (VT_ARRAY.0 | VT_BSTR.0) => WmiValue::TextList(strings(inner.Anonymous.parray)),
            _ => WmiValue::Null,
        }
    }
}

/// The strings of a one-dimensional SAFEARRAY of BSTR.
unsafe fn strings(array: *mut SAFEARRAY) -> Vec<String> {
    if array.is_null() {
        return Vec::new();
    }
    let mut items = Vec::new();
    // SAFETY: `array` is a live one-dimensional array of BSTR owned by the VARIANT; each element
    // is copied into an owned BSTR that frees itself.
    unsafe {
        let (Ok(lower), Ok(upper)) = (SafeArrayGetLBound(array, 1), SafeArrayGetUBound(array, 1)) else { return items };
        for index in lower..=upper {
            let mut item = BSTR::new();
            if SafeArrayGetElement(array, &index, (&mut item as *mut BSTR).cast()).is_ok() {
                items.push(item.to_string());
            }
        }
    }
    items
}
