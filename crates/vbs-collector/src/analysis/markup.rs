//! A lenient scanner for XML and HTML: Windows Script Files (.wsf), script components (.wsc),
//! HTML applications (.hta) and task definitions. It never fails – broken markup just yields
//! fewer tags. The content of `<script>` and `<style>` elements is not scanned for tags
//! (`If a<b Then` is code, not markup).

use std::borrow::Cow;

/// One start or end tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag<'a> {
    /// Element name as written, e.g. `script`, `HTA:APPLICATION`, `Exec`.
    pub name: &'a str,
    pub closing: bool,
    pub self_closing: bool,
    /// Attribute names as written with entity-decoded values.
    pub attributes: Vec<(&'a str, String)>,
    /// Byte offset of `<`.
    pub start: usize,
    /// Byte offset just after `>`.
    pub end: usize,
}

impl Tag<'_> {
    /// Case-insensitive comparison of the element name (ignoring a namespace prefix).
    pub fn is(&self, name: &str) -> bool {
        let local = self.name.rsplit(':').next().unwrap_or(self.name);
        local.eq_ignore_ascii_case(name) || self.name.eq_ignore_ascii_case(name)
    }

    /// Value of an attribute (case-insensitive name).
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

/// All tags of `text` in document order.
pub fn tags(text: &str) -> Vec<Tag<'_>> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(offset) = text[pos..].find('<') {
        let start = pos + offset;
        let rest = &text[start..];
        let skip_to =
            |marker: &str, from: usize| text[from..].find(marker).map_or(text.len(), |i| from + i + marker.len());
        if rest.starts_with("<!--") {
            pos = skip_to("-->", start + 4);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            pos = skip_to("]]>", start + 9);
            continue;
        }
        if rest.starts_with("<?") {
            pos = skip_to("?>", start + 2);
            continue;
        }
        if rest.starts_with("<!") {
            pos = skip_to(">", start + 2);
            continue;
        }
        let Some(tag) = parse_tag(text, start) else {
            pos = start + 1;
            continue;
        };
        pos = tag.end;
        let raw_text = !tag.closing && !tag.self_closing && (tag.is("script") || tag.is("style"));
        let element = tag.name.to_owned();
        found.push(tag);
        if raw_text {
            // Jump to the matching end tag; everything in between is code.
            let end = find_ignore_case(text, pos, &format!("</{element}"));
            pos = end.unwrap_or(bytes.len());
        }
    }
    found
}

fn parse_tag(text: &str, start: usize) -> Option<Tag<'_>> {
    let bytes = text.as_bytes();
    let mut i = start + 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && is_name_byte(bytes[i]) {
        i += 1;
    }
    if i == name_start || !bytes[name_start].is_ascii_alphabetic() {
        return None;
    }
    let name = &text[name_start..i];
    let mut attributes = Vec::new();
    let mut self_closing = false;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => return None,
            Some(b'>') => {
                i += 1;
                break;
            }
            Some(b'/') if bytes.get(i + 1) == Some(&b'>') => {
                self_closing = true;
                i += 2;
                break;
            }
            Some(b'/') => i += 1,
            Some(_) => {}
        }
        if i >= bytes.len() || bytes[i] == b'>' {
            continue;
        }
        let key_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let key = &text[key_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            match bytes.get(i) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let value_start = i + 1;
                    let value_end = text[value_start..].find(char::from(quote)).map_or(text.len(), |n| value_start + n);
                    value = decode_entities(&text[value_start..value_end]).into_owned();
                    i = (value_end + 1).min(text.len());
                }
                Some(_) => {
                    let value_start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value = decode_entities(&text[value_start..i]).into_owned();
                }
                None => {}
            }
        }
        if key.is_empty() {
            i += 1; // stray character – keep scanning
        } else {
            attributes.push((key, value));
        }
    }
    Some(Tag { name, closing, self_closing, attributes, start, end: i })
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.' | b'-')
}

fn find_ignore_case(text: &str, from: usize, needle: &str) -> Option<usize> {
    let haystack = text.as_bytes();
    let needle = needle.as_bytes();
    (from..haystack.len().saturating_sub(needle.len() - 1))
        .find(|&i| haystack[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

/// Replaces `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and numeric character references.
pub fn decode_entities(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find(';').filter(|&end| end <= 10);
        let decoded = end.and_then(|end| {
            let entity = &after[..end];
            let c = match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let number = entity.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Text between the end of `open` and the next tag, entity-decoded and trimmed.
pub fn text_after<'a>(text: &'a str, open: &Tag<'_>, next: Option<&Tag<'_>>) -> Cow<'a, str> {
    let end = next.map_or(text.len(), |tag| tag.start);
    let raw = text.get(open.end..end).unwrap_or_default();
    let raw = raw.trim();
    let raw = raw.strip_prefix("<![CDATA[").and_then(|inner| inner.strip_suffix("]]>")).unwrap_or(raw);
    decode_entities(raw)
}

/// A `<script>` element: its declared language and the byte range of its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptBlock {
    /// `language` or `type` attribute, e.g. `VBScript`, `JScript`, `text/vbscript`.
    pub language: Option<String>,
    /// `src` attribute of an external script.
    pub src: Option<String>,
    /// Byte range of the element's content (between the tags).
    pub content: std::ops::Range<usize>,
}

impl ScriptBlock {
    /// VBScript or encoded VBScript (`VBScript.Encode`).
    pub fn is_vbscript(&self) -> bool {
        self.language.as_deref().is_some_and(|language| language.to_ascii_lowercase().contains("vbscript"))
    }
}

/// All `<script>` elements of a document.
pub fn script_blocks(text: &str) -> Vec<ScriptBlock> {
    let tags = tags(text);
    let mut blocks = Vec::new();
    for (index, tag) in tags.iter().enumerate() {
        if tag.closing || !tag.is("script") {
            continue;
        }
        let end = if tag.self_closing {
            tag.end
        } else {
            tags.get(index + 1).filter(|next| next.closing && next.is("script")).map_or(text.len(), |next| next.start)
        };
        let language = tag.attribute("language").or_else(|| tag.attribute("type")).map(str::to_owned);
        blocks.push(ScriptBlock { language, src: tag.attribute("src").map(str::to_owned), content: tag.end..end });
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tags_and_attributes() {
        let text = r#"<?xml version="1.0"?><!-- <script language="VBScript"> --><job id='a'><script language=VBScript src="x.vbs"/><br></job>"#;
        let tags = tags(text);
        let names: Vec<_> = tags.iter().map(|t| (t.name, t.closing)).collect();
        assert_eq!(names, [("job", false), ("script", false), ("br", false), ("job", true)]);
        assert_eq!(tags[0].attribute("ID"), Some("a"));
        assert_eq!(tags[1].attribute("language"), Some("VBScript"));
        assert!(tags[1].self_closing);
    }

    #[test]
    fn script_content_is_not_markup() {
        let text =
            "<script language=\"VBScript\">\r\nIf a<b Then document.write(\"<b>x</b>\")\r\n</SCRIPT><p onclick='x'>";
        let blocks = script_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].is_vbscript());
        assert!(text[blocks[0].content.clone()].contains("If a<b Then"));
        assert!(tags(text).iter().any(|t| t.is("p") && t.attribute("onclick") == Some("x")));
    }

    #[test]
    fn decodes_entities_and_text() {
        assert_eq!(
            decode_entities("a &amp;&amp; b &lt;x&gt; &quot;q&quot; &#65;&#x42; &bogus; &"),
            "a && b <x> \"q\" AB &bogus; &"
        );
        let text =
            "<Command>C:\\Windows\\System32\\wscript.exe</Command><Arguments>&quot;C:\\x y\\a.vbs&quot;</Arguments>";
        let found = tags(text);
        assert_eq!(text_after(text, &found[2], found.get(3)), "\"C:\\x y\\a.vbs\"");
        assert!(tags("a < b and <1 not a tag").is_empty());
    }
}
