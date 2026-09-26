//! Read-only event log view (Windows Event Log API, `wevtapi.dll`): each query runs once and
//! returns rendered events; nothing is subscribed, exported, cleared or written.

use std::ptr;

use vbs_core::views::{ChannelInfo, EventLogView, EventRecord, ViewError};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND,
    GetLastError,
};
use windows_sys::Win32::System::EventLog::{
    EVT_HANDLE, EVT_VARIANT, EvtClose, EvtGetLogInfo, EvtLogNumberOfLogRecords, EvtNext, EvtOpenChannelPath,
    EvtOpenLog, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender, EvtRenderEventXml,
};

use crate::analysis::event_xml::parse_event;

/// `ERROR_EVT_CHANNEL_NOT_FOUND`
const CHANNEL_NOT_FOUND: u32 = 15007;
/// `ERROR_EVT_INVALID_QUERY`
const INVALID_QUERY: u32 = 15001;
/// Events fetched per `EvtNext` call.
const BATCH: usize = 64;
/// How long one `EvtNext` call may wait (local logs answer in milliseconds).
const TIMEOUT_MS: u32 = 30_000;

#[derive(Debug, Default, Clone, Copy)]
pub struct WinEventLogs;

/// An event log handle, closed on drop.
struct Handle(EVT_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: the handle came from a successful Evt* call and is closed exactly once.
            unsafe { EvtClose(self.0) };
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn last_error() -> ViewError {
    // SAFETY: reads the calling thread's last-error value.
    match unsafe { GetLastError() } {
        CHANNEL_NOT_FOUND | ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => ViewError::NotFound,
        ERROR_ACCESS_DENIED => ViewError::AccessDenied,
        INVALID_QUERY => ViewError::Failed("invalid query".into()),
        other => ViewError::Failed(format!("event log error {other}")),
    }
}

impl EventLogView for WinEventLogs {
    fn channel(&self, channel: &str) -> Result<ChannelInfo, ViewError> {
        let path = wide(channel);
        // SAFETY: `path` is NUL-terminated; the local session (0) is used; read access only.
        let log = Handle(unsafe { EvtOpenLog(0, path.as_ptr(), EvtOpenChannelPath) });
        if log.0 == 0 {
            return Err(last_error());
        }
        // SAFETY: EVT_VARIANT is plain data; EvtGetLogInfo fills at most the size we pass.
        let records = unsafe {
            let mut value = std::mem::zeroed::<EVT_VARIANT>();
            let mut used = 0u32;
            let ok =
                EvtGetLogInfo(log.0, EvtLogNumberOfLogRecords, size_of::<EVT_VARIANT>() as u32, &mut value, &mut used);
            (ok != 0).then_some(value.Anonymous.UInt64Val)
        };
        let oldest = query(channel, "*", 1, false)?.first().and_then(|r| r.time);
        let newest = query(channel, "*", 1, true)?.first().and_then(|r| r.time);
        Ok(ChannelInfo { records, oldest, newest })
    }

    fn query(&self, channel: &str, xpath: &str, max: usize) -> Result<Vec<EventRecord>, ViewError> {
        query(channel, xpath, max, true)
    }
}

fn query(channel: &str, xpath: &str, max: usize, newest_first: bool) -> Result<Vec<EventRecord>, ViewError> {
    let path = wide(channel);
    let text = wide(xpath);
    let flags = EvtQueryChannelPath | if newest_first { EvtQueryReverseDirection } else { 0 };
    // SAFETY: both strings are NUL-terminated and outlive the call; the local session (0) is used.
    let results = Handle(unsafe { EvtQuery(0, path.as_ptr(), text.as_ptr(), flags) });
    if results.0 == 0 {
        return Err(last_error());
    }
    let mut records = Vec::new();
    let mut handles: [EVT_HANDLE; BATCH] = [0; BATCH];
    while records.len() < max {
        let wanted = BATCH.min(max - records.len()) as u32;
        let mut returned = 0u32;
        // SAFETY: `handles` holds `wanted` (≤ BATCH) entries; each returned handle is closed below.
        let ok = unsafe { EvtNext(results.0, wanted, handles.as_mut_ptr(), TIMEOUT_MS, 0, &mut returned) };
        if ok == 0 {
            // SAFETY: reads the calling thread's last-error value.
            let error = unsafe { GetLastError() };
            if error == ERROR_NO_MORE_ITEMS {
                break;
            }
            return Err(ViewError::Failed(format!("event log error {error}")));
        }
        for &event in &handles[..returned as usize] {
            let event = Handle(event);
            if let Some(xml) = render(event.0) {
                records.push(parse_event(&xml));
            }
        }
    }
    Ok(records)
}

/// The event as XML.
fn render(event: EVT_HANDLE) -> Option<String> {
    let mut used = 0u32;
    let mut count = 0u32;
    // SAFETY: a null buffer of size 0 asks for the required size.
    let ok = unsafe { EvtRender(0, event, EvtRenderEventXml, 0, ptr::null_mut(), &mut used, &mut count) };
    // SAFETY: reads the calling thread's last-error value.
    if ok == 0 && unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return None;
    }
    let mut buffer = vec![0u16; (used as usize).div_ceil(2) + 1];
    let size = (buffer.len() * 2) as u32;
    // SAFETY: `buffer` holds `size` bytes.
    let ok = unsafe { EvtRender(0, event, EvtRenderEventXml, size, buffer.as_mut_ptr().cast(), &mut used, &mut count) };
    if ok == 0 {
        return None;
    }
    let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}
