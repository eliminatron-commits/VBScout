//! Read-only registry view. Keys are opened with `KEY_READ` and closed again;
//! there is no code path that creates, changes, loads or deletes anything.

use std::ptr;

use vbs_core::views::{Bitness, Hive, RegValue, RegistryView, ViewError};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ, KEY_WOW64_64KEY, REG_BINARY, REG_DWORD,
    REG_EXPAND_SZ, REG_MULTI_SZ, REG_QWORD, REG_SZ, RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW,
    RegQueryInfoKeyW,
};

/// Largest value the view reads (registry values are small; this bounds memory on odd data).
const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct WinRegistry;

/// An open key, closed on drop.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful RegOpenKeyExW and is closed exactly once.
        unsafe { RegCloseKey(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn check(status: WIN32_ERROR) -> Result<(), ViewError> {
    match status {
        ERROR_SUCCESS => Ok(()),
        ERROR_FILE_NOT_FOUND => Err(ViewError::NotFound),
        ERROR_ACCESS_DENIED => Err(ViewError::AccessDenied),
        other => Err(ViewError::Failed(format!("registry error {other}"))),
    }
}

fn open(hive: Hive, path: &str, bitness: Bitness) -> Result<Key, ViewError> {
    let root = match hive {
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::Users => HKEY_USERS,
    };
    let (path, view) = view_path(hive, path.trim_matches('\\'), bitness);
    let path = wide(&path);
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: `path` is NUL-terminated and outlives the call; on success `key`
    // receives a handle opened with read access only, owned by `Key`.
    let status = unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ | view, &mut key) };
    check(status)?;
    Ok(Key(key))
}

/// The key path to open for a view, and the WOW64 flag it needs.
///
/// The 64-bit collector reads both views without WOW64 flags: the 64-bit view is its own, and the
/// 32-bit view of `HKLM\SOFTWARE` – the part WOW64 redirects – is the physical key
/// `SOFTWARE\WOW6432Node`; everything else is shared by both views. A view flag would make
/// advapi32 tag the parent handle (`NtSetInformationKey`, `KeySetHandleTagsInformation`): no
/// change to the registry, but a "set" operation in a kernel trace, and the read-only proof counts
/// every one of them. Keys that WOW64 shares under `SOFTWARE` have no `WOW6432Node` twin, so
/// their values are not reported twice.
fn view_path(hive: Hive, path: &str, bitness: Bitness) -> (String, u32) {
    if cfg!(target_pointer_width = "32") {
        // A 32-bit build (not shipped) is redirected itself and needs the flag for the 64-bit view.
        return (path.to_owned(), if bitness == Bitness::Native { KEY_WOW64_64KEY } else { 0 });
    }
    let software = path.get(..8).is_some_and(|head| head.eq_ignore_ascii_case("SOFTWARE"))
        && matches!(path.as_bytes().get(8), None | Some(b'\\'));
    match (bitness, hive) {
        (Bitness::Wow32, Hive::LocalMachine) if software => (format!(r"SOFTWARE\WOW6432Node{}", &path[8..]), 0),
        _ => (path.to_owned(), 0),
    }
}

/// Sizes reported by RegQueryInfoKeyW (in characters for names, bytes for data).
struct KeyInfo {
    max_subkey_name: u32,
    max_value_name: u32,
    max_value_data: u32,
}

fn query_info(key: &Key) -> Result<KeyInfo, ViewError> {
    let (mut subkeys, mut max_subkey_name, mut values, mut max_value_name, mut max_value_data) = (0, 0, 0, 0, 0);
    // SAFETY: all out-pointers point to live locals; unused outputs are null as documented.
    let status = unsafe {
        RegQueryInfoKeyW(
            key.0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
            &mut subkeys,
            &mut max_subkey_name,
            ptr::null_mut(),
            &mut values,
            &mut max_value_name,
            &mut max_value_data,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    check(status)?;
    Ok(KeyInfo { max_subkey_name, max_value_name, max_value_data })
}

fn utf16_bytes(data: &[u8]) -> Vec<u16> {
    data.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect()
}

fn decode(kind: u32, data: &[u8]) -> RegValue {
    let text = || {
        let units = utf16_bytes(data);
        let end = units.iter().position(|&unit| unit == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    };
    match kind {
        REG_SZ => RegValue::Text(text()),
        REG_EXPAND_SZ => RegValue::ExpandText(text()),
        REG_MULTI_SZ => RegValue::MultiText(
            utf16_bytes(data)
                .split(|&unit| unit == 0)
                .filter(|part| !part.is_empty())
                .map(String::from_utf16_lossy)
                .collect(),
        ),
        REG_DWORD if data.len() >= 4 => RegValue::Dword(u32::from_le_bytes([data[0], data[1], data[2], data[3]])),
        REG_QWORD if data.len() >= 8 => {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&data[..8]);
            RegValue::Qword(u64::from_le_bytes(bytes))
        }
        REG_BINARY => RegValue::Binary(data.to_vec()),
        other => RegValue::Other(other),
    }
}

impl RegistryView for WinRegistry {
    fn subkeys(&self, hive: Hive, path: &str, bitness: Bitness) -> Result<Vec<String>, ViewError> {
        let key = open(hive, path, bitness)?;
        let info = query_info(&key)?;
        let mut names = Vec::new();
        let mut buffer = vec![0u16; info.max_subkey_name as usize + 1];
        let mut index = 0;
        loop {
            let mut len = buffer.len() as u32;
            // SAFETY: `buffer` holds `len` characters; the remaining outputs are optional and null.
            let status = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut len,
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            match status {
                ERROR_SUCCESS => names.push(String::from_utf16_lossy(&buffer[..len as usize])),
                ERROR_NO_MORE_ITEMS => break,
                // A longer name appeared since RegQueryInfoKeyW: grow and retry the same index.
                ERROR_MORE_DATA if buffer.len() < 32_768 => {
                    buffer.resize(buffer.len() * 2, 0);
                    continue;
                }
                other => return check(other).map(|()| names),
            }
            index += 1;
        }
        Ok(names)
    }

    fn values(&self, hive: Hive, path: &str, bitness: Bitness) -> Result<Vec<(String, RegValue)>, ViewError> {
        let key = open(hive, path, bitness)?;
        let info = query_info(&key)?;
        let mut values = Vec::new();
        let mut name = vec![0u16; info.max_value_name as usize + 1];
        let mut data = vec![0u8; (info.max_value_data as usize).clamp(8, MAX_VALUE_BYTES)];
        let mut index = 0;
        loop {
            let mut name_len = name.len() as u32;
            let mut data_len = data.len() as u32;
            let mut kind = 0u32;
            // SAFETY: `name` holds `name_len` characters and `data` holds `data_len` bytes; both outlive the call.
            let status = unsafe {
                RegEnumValueW(
                    key.0,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut data_len,
                )
            };
            match status {
                ERROR_SUCCESS => {
                    let value_name = String::from_utf16_lossy(&name[..name_len as usize]);
                    values.push((value_name, decode(kind, &data[..data_len as usize])));
                }
                ERROR_NO_MORE_ITEMS => break,
                ERROR_MORE_DATA => {
                    let wanted = (data_len as usize).max(data.len() * 2);
                    if wanted > MAX_VALUE_BYTES || name.len() >= 32_768 {
                        // Larger than any value a module needs: skip it, keep the others.
                        index += 1;
                        continue;
                    }
                    data.resize(wanted, 0);
                    name.resize((name.len() * 2).min(32_768), 0);
                    continue;
                }
                other => return check(other).map(|()| values),
            }
            index += 1;
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_32_bit_view_is_read_through_wow6432node_without_view_flags() {
        let run = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
        assert_eq!(view_path(Hive::LocalMachine, run, Bitness::Native), (run.to_owned(), 0));
        assert_eq!(
            view_path(Hive::LocalMachine, run, Bitness::Wow32),
            (r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run".to_owned(), 0)
        );
        assert_eq!(view_path(Hive::LocalMachine, "software", Bitness::Wow32), (r"SOFTWARE\WOW6432Node".to_owned(), 0));
        // Shared parts of the registry are the same in both views.
        let services = r"SYSTEM\CurrentControlSet\Services";
        assert_eq!(view_path(Hive::LocalMachine, services, Bitness::Wow32), (services.to_owned(), 0));
        assert_eq!(view_path(Hive::LocalMachine, "SOFTWAREX", Bitness::Wow32), ("SOFTWAREX".to_owned(), 0));
        assert_eq!(view_path(Hive::Users, r"S-1-5-18\Software", Bitness::Wow32), (r"S-1-5-18\Software".to_owned(), 0));
    }

    #[test]
    fn reads_the_32_bit_view_of_this_machine() {
        // Every 64-bit Windows has the WOW6432Node twin of the Windows key.
        let key = r"SOFTWARE\Microsoft\Windows\CurrentVersion";
        assert!(WinRegistry.values(Hive::LocalMachine, key, Bitness::Wow32).is_ok());
        assert!(WinRegistry.subkeys(Hive::LocalMachine, key, Bitness::Native).unwrap().iter().any(|k| k == "Run"));
    }
}
