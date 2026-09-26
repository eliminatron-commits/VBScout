//! Read-only registry view. Keys are opened with `KEY_READ` (plus the WOW64
//! view flag) and closed again; there is no code path that creates, changes,
//! loads or deletes anything.

use std::ptr;

use vbs_core::views::{Bitness, Hive, RegValue, RegistryView, ViewError};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_BINARY,
    REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_QWORD, REG_SZ, RegCloseKey, RegEnumKeyExW, RegEnumValueW,
    RegOpenKeyExW, RegQueryInfoKeyW,
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
    // The 64-bit collector sees the native view by default. Passing KEY_WOW64_64KEY anyway
    // makes advapi32 tag the handle (NtSetInformationKey), which a kernel trace rightly lists
    // as a "set" operation – so the flag is only used where it changes the view.
    let view = match bitness {
        Bitness::Native if cfg!(target_pointer_width = "64") => 0,
        Bitness::Native => KEY_WOW64_64KEY,
        Bitness::Wow32 => KEY_WOW64_32KEY,
    };
    let path = wide(path.trim_matches('\\'));
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: `path` is NUL-terminated and outlives the call; on success `key`
    // receives a handle opened with read access only, owned by `Key`.
    let status = unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ | view, &mut key) };
    check(status)?;
    Ok(Key(key))
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
