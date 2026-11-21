//! 歌词获取：同名 `.lrc` 文件（主路径）+ 音频文件内嵌歌词。
//!
//! - **同名 .lrc**：与歌曲同目录、同名（stem 相同、扩展名为 `.lrc`，大小写不敏感）。
//!   导入歌曲时会随音频一并复制进曲库（见 `import` 模块），故曲库内可直接命中。
//! - **内嵌歌词**：
//!   - FLAC / OGG：Vorbis Comment 的 `LYRICS` / `UNSYNCEDLYRICS`（经 lofty 的 `ItemKey::Unknown` 读取）；
//!   - MP3 / AAC / APE 等：ID3v2 的 `USLT`（未同步歌词帧）——lofty 通用标签会丢弃该帧，
//!     故此处自行解析 ID3v2 标签（仅需一个帧，实现精简）。
//!
//! 文本编码：UTF-8 / UTF-16(BOM) / GBK-GB18030（中文 lrc 常见），见 `decode_bytes`。
use lofty::prelude::*;
use lofty::probe::Probe;
use std::path::{Path, PathBuf};

/// 歌词来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsSource {
    /// 同名 `.lrc` 文件。
    Sidecar,
    /// 音频文件内嵌歌词。
    Embedded,
}

impl LyricsSource {
    /// 对外标识（前端展示 / API 返回）。
    pub fn as_str(self) -> &'static str {
        match self {
            LyricsSource::Sidecar => "lrc",
            LyricsSource::Embedded => "embedded",
        }
    }
}

/// 一首歌的歌词。
#[derive(Debug, Clone)]
pub struct Lyrics {
    /// 来源。
    pub source: LyricsSource,
    /// 原始文本（LRC 原文或纯文本歌词）。
    pub text: String,
    /// 是否带时间轴（形如 `[00:12.34]`）。
    pub timed: bool,
}

/// 查找歌曲同目录同名的 `.lrc` 文件。
///
/// 先精确匹配 `<stem>.lrc`；再在同目录做大小写不敏感匹配（Windows 上不敏感，
/// 但从其他平台拷贝过来的曲库可能出现大小写不一致）。
pub fn find_sidecar(audio_path: &Path) -> Option<PathBuf> {
    let dir = audio_path.parent()?;
    let stem = audio_path.file_stem()?.to_str()?;

    let exact = dir.join(format!("{stem}.lrc"));
    if exact.is_file() {
        return Some(exact);
    }
    let lower = stem.to_lowercase();
    for entry in std::fs::read_dir(dir).ok()? {
        let Ok(entry) = entry else { continue };
        let p = entry.path();
        let is_lrc = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("lrc"))
            .unwrap_or(false);
        if !is_lrc {
            continue;
        }
        let same_stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase() == lower)
            .unwrap_or(false);
        if same_stem {
            return Some(p);
        }
    }
    None
}

/// 读取歌词文本文件（自动处理 BOM / UTF-8 / GBK）。
pub fn read_text_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = decode_bytes(&bytes);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 字节流 → 文本：UTF-8 优先，失败回退 GB18030（兼容 GBK）；支持 UTF-8/UTF-16 BOM。
pub fn decode_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        // 中文 lrc 常见 GBK/GB18030；GB18030 是 GBK 的超集。
        Err(_) => encoding_rs::GB18030.decode(bytes).0.into_owned(),
    }
}

/// 提取音频文件内嵌的歌词。
pub fn extract_embedded(path: &Path) -> Option<String> {
    // ID3v2 的 USLT 帧（MP3 / AAC / APE 等带 ID3v2 的容器）。
    if let Some(s) = read_id3v2_uslt(path) {
        let t = s.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    // Vorbis Comment 的 LYRICS / UNSYNCEDLYRICS（FLAC / OGG）。
    embedded_via_lofty(path)
}

/// 经 lofty 通用标签读取内嵌歌词（Vorbis Comment 的 LYRICS 类键）。
fn embedded_via_lofty(path: &Path) -> Option<String> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    for tag in tagged.tags() {
        for item in tag.items() {
            let ItemKey::Unknown(k) = item.key() else { continue };
            if !matches!(
                k.to_ascii_uppercase().as_str(),
                "LYRICS" | "UNSYNCEDLYRICS" | "LYRIC" | "LYRICSUSLT" | "UNSYNCEDLYRIC"
            ) {
                continue;
            }
            let Some(text) = item.value().text() else { continue };
            let t = text.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// 读取音频文件开头并解析 ID3v2 的 USLT 帧。
fn read_id3v2_uslt(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 10];
    f.read_exact(&mut header).ok()?;
    if &header[0..3] != b"ID3" {
        return None;
    }
    let tag_size = synchsafe(&header[6..10])? as usize;
    if tag_size == 0 {
        return None;
    }
    let mut body = vec![0u8; tag_size];
    f.read_exact(&mut body).ok()?;
    let mut all = Vec::with_capacity(10 + tag_size);
    all.extend_from_slice(&header);
    all.extend_from_slice(&body);
    parse_id3v2_uslt(&all)
}

/// 解析 ID3v2 标签中的第一个 USLT 帧。
fn parse_id3v2_uslt(data: &[u8]) -> Option<String> {
    if data.len() < 10 || &data[0..3] != b"ID3" {
        return None;
    }
    let version_major = data[3];
    let flags = data[5];
    let tag_size = synchsafe(&data[6..10])? as usize;
    let end = std::cmp::min(10usize.saturating_add(tag_size), data.len());
    let mut body: Vec<u8> = data[10..end].to_vec();

    // 反同步：0xFF 0x00 → 0xFF（仅在 header flags 置位时处理）。
    if flags & 0x80 != 0 {
        body = de_unsynchronise(&body);
    }
    // 扩展头。
    if flags & 0x40 != 0 && body.len() >= 4 {
        let ext_size = if version_major >= 4 {
            synchsafe(&body[0..4])? as usize
        } else {
            u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize + 4
        };
        if ext_size > body.len() {
            return None;
        }
        body.drain(..ext_size);
    }

    // 逐帧扫描。
    let mut pos = 0usize;
    while pos + 10 <= body.len() {
        let id = [body[pos], body[pos + 1], body[pos + 2], body[pos + 3]];
        if id[0] == 0 {
            break; // 填充区，结束。
        }
        let size = if version_major >= 4 {
            synchsafe(&body[pos + 4..pos + 8])? as usize
        } else {
            u32::from_be_bytes([
                body[pos + 4],
                body[pos + 5],
                body[pos + 6],
                body[pos + 7],
            ]) as usize
        };
        let start = pos + 10;
        let stop = match start.checked_add(size) {
            Some(s) if s <= body.len() => s,
            _ => break,
        };
        if &id == b"USLT" {
            return parse_uslt_frame(&body[start..stop]);
        }
        pos = stop;
    }
    None
}

/// 解析 USLT 帧内容：编码字节 + 3 字节语言 + 以终止符结束的内容描述符 + 歌词正文。
fn parse_uslt_frame(frame: &[u8]) -> Option<String> {
    if frame.len() < 5 {
        return None;
    }
    let encoding = frame[0];
    let wide = encoding == 1 || encoding == 2;
    let mut i = 4usize; // 跳过编码字节 + 3 字节语言码
    while i < frame.len() {
        if wide {
            if i + 1 >= frame.len() {
                break;
            }
            if frame[i] == 0 && frame[i + 1] == 0 {
                i += 2;
                break;
            }
            i += 2;
        } else {
            if frame[i] == 0 {
                i += 1;
                break;
            }
            i += 1;
        }
    }
    if i > frame.len() {
        return None;
    }
    let text = decode_id3_text(encoding, &frame[i..])?;
    let t = text.trim_matches('\u{0}').trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// ID3 文本编码解码：0=ISO-8859-1 / 1=UTF-16(BOM) / 2=UTF-16BE / 3=UTF-8。
fn decode_id3_text(encoding: u8, bytes: &[u8]) -> Option<String> {
    match encoding {
        0 => Some(bytes.iter().map(|&b| b as char).collect()),
        1 => {
            if bytes.len() < 2 {
                return None;
            }
            let (little, rest) = if bytes[0] == 0xFF && bytes[1] == 0xFE {
                (true, &bytes[2..])
            } else if bytes[0] == 0xFE && bytes[1] == 0xFF {
                (false, &bytes[2..])
            } else {
                (true, bytes)
            };
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|c| {
                    if little {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    }
                })
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        2 => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => Some(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// ID3v2 的 synchsafe 整数（每字节仅低 7 位有效）。
fn synchsafe(b: &[u8]) -> Option<u32> {
    if b.len() < 4 {
        return None;
    }
    Some(((b[0] as u32 & 0x7F) << 21)
        | ((b[1] as u32 & 0x7F) << 14)
        | ((b[2] as u32 & 0x7F) << 7)
        | (b[3] as u32 & 0x7F))
}

/// 反同步：还原 0xFF 0x00 → 0xFF。
fn de_unsynchronise(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0usize;
    while i < data.len() {
        out.push(data[i]);
        if data[i] == 0xFF && i + 1 < data.len() && data[i + 1] == 0x00 {
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

/// 判断文本是否为带时间轴的 LRC（含形如 `[00:12.34]` 或 `[00:12]` 的时间戳行）。
pub fn is_timed_lrc(text: &str) -> bool {
    text.lines().any(|line| {
        let t = line.trim_start();
        if !t.starts_with('[') {
            return false;
        }
        let Some(close) = t.find(']') else { return false };
        let ts = &t[1..close];
        let mut parts = ts.split(':');
        let Some(min) = parts.next() else { return false };
        let Some(sec) = parts.next() else { return false };
        let min = min.trim();
        let sec = sec.trim();
        !min.is_empty()
            && min.chars().all(|c| c.is_ascii_digit())
            && sec.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
    })
}

/// 获取歌曲歌词：优先同名 `.lrc`，其次内嵌歌词。
pub fn get_lyrics(audio_path: &Path) -> Option<Lyrics> {
    if let Some(lrc) = find_sidecar(audio_path) {
        if let Some(text) = read_text_file(&lrc) {
            let timed = is_timed_lrc(&text);
            return Some(Lyrics { source: LyricsSource::Sidecar, text, timed });
        }
    }
    let text = extract_embedded(audio_path)?;
    let timed = is_timed_lrc(&text);
    Some(Lyrics { source: LyricsSource::Embedded, text, timed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_lrc_detection() {
        let timed = "[00:12.34]第一句\n[00:15.00]第二句";
        assert!(is_timed_lrc(timed));
        let untimed = "第一句\n第二句";
        assert!(!is_timed_lrc(untimed));
        // 元信息行不应被误判为时间轴。
        let meta = "[ti:歌名]\n[ar:歌手]\n作词 : 某某";
        assert!(!is_timed_lrc(meta));
        // 带毫秒与多时间戳的行。
        assert!(is_timed_lrc("[00:12.34][00:20.00]副歌"));
    }

    #[test]
    fn decode_gbk_bytes() {
        // “中文歌词” 的 GBK 编码：中=D6D0 文=CEC4 歌=B8E8 词=B4CA。
        let gbk: Vec<u8> = vec![0xD6, 0xD0, 0xCE, 0xC4, 0xB8, 0xE8, 0xB4, 0xCA];
        assert_eq!(decode_bytes(&gbk), "中文歌词");
    }

    #[test]
    fn decode_utf8_and_bom() {
        assert_eq!(decode_bytes("歌词".as_bytes()), "歌词");
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("歌词".as_bytes());
        assert_eq!(decode_bytes(&bom), "歌词");
    }

    #[test]
    fn parse_id3v2_uslt_frame_utf8() {
        // 构造最小 ID3v2.3 标签：USLT(UTF-8) 帧。
        let mut frame = vec![3u8]; // 编码 = UTF-8
        frame.extend_from_slice(b"chi"); // 语言
        frame.push(0); // 内容描述符终止符
        frame.extend_from_slice("你好世界".as_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(b"USLT");
        body.extend_from_slice(&(frame.len() as u32).to_be_bytes());
        body.extend_from_slice(&[0, 0]); // flags
        body.extend_from_slice(&frame);

        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3");
        tag.extend_from_slice(&[3, 0]); // version
        tag.push(0); // flags
        let size = body.len() as u32;
        tag.extend_from_slice(&[
            ((size >> 21) & 0x7F) as u8,
            ((size >> 14) & 0x7F) as u8,
            ((size >> 7) & 0x7F) as u8,
            (size & 0x7F) as u8,
        ]);
        tag.extend_from_slice(&body);

        assert_eq!(parse_id3v2_uslt(&tag).as_deref(), Some("你好世界"));
    }

    #[test]
    fn sidecar_lookup_prefers_exact_then_case_insensitive() {
        let dir = std::env::temp_dir().join("mt_lyrics_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("Song.mp3");
        std::fs::write(&audio, b"x").unwrap();

        // 无 lrc 时返回 None。
        assert!(find_sidecar(&audio).is_none());

        // 同名 lrc（大小写可能不一致）应被命中。
        std::fs::write(dir.join("song.lrc"), b"[00:01.00]hi").unwrap();
        let found = find_sidecar(&audio).expect("应找到同名 lrc");
        let fname = found.file_name().unwrap().to_str().unwrap().to_lowercase();
        assert_eq!(fname, "song.lrc");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
