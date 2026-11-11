//! 音频元数据采集（lofty，纯 Rust）。对应 docs/03 §5。
//! 抽取采样率/位深/码率/声道/时长，并尽力提取内嵌专辑封面。
use crate::Result;
use lofty::prelude::*;
use lofty::probe::Probe;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct AudioMeta {
    pub format: String,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub bitrate: Option<u32>,
    pub channels: Option<u8>,
    pub duration_ms: Option<u64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// #28：曲级流派 / 发行年份（来自音频标签）。`genre` 为 None 表示「标签无此字段」；
    /// 入库时统一写成空串以标记「已尝试抽取」，供扫描增量门控区分从未抽取的旧记录。
    pub genre: Option<String>,
    pub year: Option<i64>,
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// 抽取文件音质元数据与基础标签。
pub fn extract(path: &Path) -> Result<AudioMeta> {
    let mut meta = AudioMeta {
        format: ext_of(path),
        ..Default::default()
    };
    let tagged = Probe::open(path).map_err(|e| anyhow::anyhow!("lofty open: {e}"))?.read()?;
    let props = tagged.properties();
    meta.sample_rate = props.sample_rate();
    meta.channels = props.channels();
    meta.duration_ms = Some(props.duration().as_millis() as u64);
    meta.bit_depth = props.bit_depth().map(|v| v as u8);
    meta.bitrate = props.audio_bitrate().map(|v| v as u32);

    for tag in tagged.tags() {
        if meta.title.is_none() {
            meta.title = tag.get_string(&ItemKey::TrackTitle).map(str::to_string);
        }
        if meta.artist.is_none() {
            meta.artist = tag.get_string(&ItemKey::TrackArtist).map(str::to_string);
        }
        if meta.album.is_none() {
            meta.album = tag.get_string(&ItemKey::AlbumTitle).map(str::to_string);
        }
        // #28：流派 / 年份。年份优先用 lofty 的 year()（数值），再回退 Year /
        // RecordingDate 文本的前四位（ID3v2 TDRC 常形如 "2021-05-01"）。
        if meta.genre.is_none() {
            meta.genre = tag
                .get_string(&ItemKey::Genre)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        if meta.year.is_none() {
            meta.year = tag.year().map(|v| v as i64).or_else(|| {
                tag.get_string(&ItemKey::Year)
                    .or_else(|| tag.get_string(&ItemKey::RecordingDate))
                    .and_then(|s| {
                        let head: String = s.trim().chars().take(4).collect();
                        head.parse::<i64>().ok()
                    })
            });
        }
        if meta.title.is_some()
            && meta.artist.is_some()
            && meta.album.is_some()
            && meta.genre.is_some()
            && meta.year.is_some()
        {
            break;
        }
    }
    Ok(meta)
}

/// 提取内嵌专辑封面（APIC）。失败/无封面返回 None。
pub fn extract_cover(path: &Path) -> Option<Vec<u8>> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    for tag in tagged.tags() {
        if let Some(pic) = tag.pictures().first() {
            return Some(pic.data().to_vec());
        }
    }
    None
}

/// 质量层级启发式（用于升级判定 / 展示）：优先按位深，其次码率，最后格式。
pub fn quality_tier_of(meta: &AudioMeta) -> String {
    if let Some(bd) = meta.bit_depth {
        if bd >= 24 {
            return "hires".to_string();
        }
        return "lossless".to_string();
    }
    if let Some(br) = meta.bitrate {
        return match br {
            0..=127 => "standard".to_string(),
            128..=191 => "higher".to_string(),
            192..=319 => "exhigh".to_string(),
            _ => "lossless".to_string(),
        };
    }
    meta.format.clone()
}
