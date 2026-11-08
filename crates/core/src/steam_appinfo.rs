//! Steam's `appcache/appinfo.vdf`: what the client knows about the apps of the account (names and
//! types also of games that are not installed). A binary file; the layout is the one SteamDB
//! documents (`SteamAppInfo`), versions 28 and 29.

use std::collections::HashMap;
use std::path::Path;

const MAGIC_28: u32 = 0x0756_4428;
const MAGIC_29: u32 = 0x0756_4429;
/// After the app ID and the size: info state, last update, PICS token, SHA-1, change number and
/// the SHA-1 of the binary data
const APP_HEADER: usize = 4 + 4 + 8 + 20 + 4 + 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    /// `game`, `dlc`, `tool`, `application`, `demo`, ... in lower case
    pub kind: String,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, count: usize) -> Option<&[u8]> {
        let slice = self.data.get(self.at..self.at.checked_add(count)?)?;
        self.at += count;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }

    fn u32(&mut self) -> Option<u32> {
        self.bytes(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Option<u64> {
        let b = self.bytes(8)?;
        Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn cstr(&mut self) -> Option<String> {
        let rest = self.data.get(self.at..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        self.at += end + 1;
        Some(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

/// Binary KeyValues: only strings are kept, nested objects are flattened to `a/b/c` paths.
fn read_object(
    reader: &mut Reader,
    keys: Option<&[String]>,
    prefix: &str,
    out: &mut HashMap<String, String>,
) -> Option<()> {
    loop {
        let kind = reader.u8()?;
        if kind == 8 {
            return Some(());
        }
        let key = match keys {
            Some(keys) => keys.get(reader.u32()? as usize)?.clone(),
            None => reader.cstr()?,
        };
        let path = if prefix.is_empty() { key.to_lowercase() } else { format!("{prefix}/{}", key.to_lowercase()) };
        match kind {
            0 => read_object(reader, keys, &path, out)?,
            1 => {
                let value = reader.cstr()?;
                out.insert(path, value);
            }
            2 | 3 | 4 | 6 => {
                reader.u32()?;
            }
            7 | 10 => {
                reader.u64()?;
            }
            5 => {
                reader.cstr()?;
            }
            _ => return None,
        }
    }
}

/// Name and type of every app in the file; empty if it cannot be read.
pub fn parse(data: &[u8]) -> HashMap<u32, AppInfo> {
    let mut apps = HashMap::new();
    let mut reader = Reader { data, at: 0 };
    let Some(magic) = reader.u32() else { return apps };
    if reader.u32().is_none() {
        return apps;
    }
    let keys: Option<Vec<String>> = match magic {
        MAGIC_28 => None,
        MAGIC_29 => {
            let Some(offset) = reader.u64().and_then(|offset| usize::try_from(offset).ok()) else { return apps };
            let mut table = Reader { data, at: offset };
            let Some(count) = table.u32() else { return apps };
            let keys: Option<Vec<String>> = (0..count).map(|_| table.cstr()).collect();
            let Some(keys) = keys else { return apps };
            Some(keys)
        }
        _ => return apps,
    };
    while let Some(app_id) = reader.u32() {
        if app_id == 0 {
            break;
        }
        let Some(size) = reader.u32().map(|size| size as usize) else { break };
        let start = reader.at;
        let mut fields = HashMap::new();
        let mut body = Reader { data, at: start + APP_HEADER };
        if read_object(&mut body, keys.as_deref(), "", &mut fields).is_some()
            && let Some(name) = fields.remove("appinfo/common/name")
        {
            let kind = fields.remove("appinfo/common/type").unwrap_or_default().to_lowercase();
            apps.insert(app_id, AppInfo { name, kind });
        }
        reader.at = start + size;
    }
    apps
}

pub fn read(path: &Path) -> HashMap<u32, AppInfo> {
    std::fs::read(path).map(|data| parse(&data)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A v29 file with the given apps (ID, name, type) and its string table.
    fn file(apps: &[(u32, &str, &str)]) -> Vec<u8> {
        let keys = ["appinfo", "common", "name", "type", "gameid"];
        let key = |name: &str| keys.iter().position(|k| *k == name).unwrap() as u32;
        let mut body = Vec::new();
        for &(app_id, name, kind) in apps {
            let mut kv = Vec::new();
            kv.push(0);
            kv.extend(key("appinfo").to_le_bytes());
            kv.push(0);
            kv.extend(key("common").to_le_bytes());
            for (field, value) in [("name", name), ("type", kind)] {
                kv.push(1);
                kv.extend(key(field).to_le_bytes());
                kv.extend(value.as_bytes());
                kv.push(0);
            }
            kv.push(2);
            kv.extend(key("gameid").to_le_bytes());
            kv.extend(7u32.to_le_bytes());
            kv.extend([8, 8, 8]);
            let mut entry = vec![0u8; APP_HEADER];
            entry.extend(kv);
            body.extend(app_id.to_le_bytes());
            body.extend((entry.len() as u32).to_le_bytes());
            body.extend(entry);
        }
        body.extend(0u32.to_le_bytes());
        let mut data = Vec::new();
        data.extend(MAGIC_29.to_le_bytes());
        data.extend(1u32.to_le_bytes());
        let table_at = 16 + body.len();
        data.extend((table_at as u64).to_le_bytes());
        data.extend(body);
        data.extend((keys.len() as u32).to_le_bytes());
        for key in keys {
            data.extend(key.as_bytes());
            data.push(0);
        }
        data
    }

    #[test]
    fn reads_names_and_types() {
        let apps = parse(&file(&[(400, "Portal", "Game"), (7, "Steam Client", "Config")]));
        assert_eq!(apps[&400], AppInfo { name: "Portal".into(), kind: "game".into() });
        assert_eq!(apps[&7].kind, "config");
    }

    #[test]
    fn broken_files_give_nothing_or_what_came_before() {
        assert!(parse(b"nonsense").is_empty());
        let mut data = file(&[(400, "Portal", "Game")]);
        data.truncate(40);
        assert!(parse(&data).is_empty());
    }
}
