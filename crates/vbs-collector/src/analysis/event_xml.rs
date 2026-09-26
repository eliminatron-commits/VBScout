//! Events as the Windows Event Log API renders them (`EvtRenderEventXml`).

use std::collections::BTreeMap;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use vbs_core::views::EventRecord;

use super::markup;

/// Provider, event ID, time, record ID and the event data of a rendered event: named
/// `<Data Name="…">` fields, unnamed ones by position (`#1`, `#2`, …), and `<UserData>` fields.
pub fn parse_event(xml: &str) -> EventRecord {
    let tags = markup::tags(xml);
    let mut record =
        EventRecord { record_id: 0, event_id: 0, provider: String::new(), time: None, data: BTreeMap::new() };
    let mut in_user_data = false;
    let mut unnamed = 0;
    for (index, tag) in tags.iter().enumerate() {
        if tag.closing {
            if tag.is("UserData") {
                in_user_data = false;
            }
            continue;
        }
        let next = tags.get(index + 1);
        let text = || markup::text_after(xml, tag, next).into_owned();
        if tag.is("Provider") {
            record.provider =
                tag.attribute("Name").or_else(|| tag.attribute("EventSourceName")).unwrap_or_default().to_owned();
        } else if tag.is("EventID") {
            record.event_id = text().parse().unwrap_or(0);
        } else if tag.is("EventRecordID") {
            record.record_id = text().parse().unwrap_or(0);
        } else if tag.is("TimeCreated") {
            record.time = tag.attribute("SystemTime").and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok());
        } else if tag.is("Data") && !tag.self_closing {
            let name = match tag.attribute("Name") {
                Some(name) => name.to_owned(),
                None => {
                    unnamed += 1;
                    format!("#{unnamed}")
                }
            };
            record.data.insert(name, text());
        } else if tag.is("UserData") {
            in_user_data = true;
        } else if in_user_data && !tag.self_closing && next.is_some_and(|n| n.closing && n.name == tag.name) {
            record.data.insert(tag.name.rsplit(':').next().unwrap_or(tag.name).to_owned(), text());
        }
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_unnamed_and_user_data() {
        let xml = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='VBScriptDeprecationAlert'/><EventID Qualifiers='0'>4096</EventID><Level>3</Level><TimeCreated SystemTime='2026-09-01T08:02:11.1234567Z'/><EventRecordID>120</EventRecordID><Channel>Application</Channel></System><EventData><Data>cscript.exe</Data><Data>cscript.exe;cmd.exe</Data><Data></Data></EventData></Event>";
        let record = parse_event(xml);
        assert_eq!(
            (record.provider.as_str(), record.event_id, record.record_id),
            ("VBScriptDeprecationAlert", 4096, 120)
        );
        assert_eq!(record.time.map(|t| t.unix_timestamp()), Some(1_788_249_731));
        assert_eq!(record.data["#1"], "cscript.exe");
        assert_eq!(record.data["#2"], "cscript.exe;cmd.exe");
        let sysmon = "<Event><System><Provider Name='Microsoft-Windows-Sysmon' Guid='{5770385f}'/><EventID>7</EventID></System><EventData><Data Name='Image'>C:\\Apps\\a.exe</Data><Data Name='ImageLoaded'>C:\\Windows\\System32\\vbscript.dll</Data></EventData></Event>";
        let record = parse_event(sysmon);
        assert_eq!(record.data["ImageLoaded"], r"C:\Windows\System32\vbscript.dll");
        let user = "<Event><System><Provider Name='X'/><EventID>1</EventID></System><UserData><Info xmlns='x'><Path>C:\\a.vbs</Path><Empty/></Info></UserData></Event>";
        assert_eq!(parse_event(user).data["Path"], r"C:\a.vbs");
    }
}
