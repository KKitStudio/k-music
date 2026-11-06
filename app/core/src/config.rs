//! 配置加载（TOML）。**双配置文件**（2026-11-04 拆分，docs/03 §3）。
//!
//! - **app 级** → `<exe 目录>/kqk-app.toml`：应用自身设置（NCM 工具、本地网易云导入源、扫描/导入策略、
//!   服务器、界面偏好等），**外加「当前曲库路径」**（`last_library_dir`）——这是唯一放在 app 目录的
//!   「曲库相关」项，因为应用必须先知道当前曲库才能找到它的配置。
//! - **曲库级** → `<曲库>/.mt/config.toml`：只存与**该曲库**相关的设置：曲库名称、文件结构方案、
//!   目录布局（download 体系、tools、app_data）。
//!
//! 运行期使用 `Config`（两份配置的合并视图），调用方（serve / library / import / db / log）无感知。
//! 持久化时按 `split()` 拆回两份分别写盘；`save_app()` 写 app 目录，`save_library()` 写 `.mt/`。
use crate::db;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::path::PathBuf;

/// app 级配置文件名（位于 exe 所在目录）。
pub const APP_CONFIG_FILE: &str = "kqk-app.toml";
/// 旧版 launcher 文件名（仅用于一次性迁移到 app 配置的 `last_library_dir`）。
const LEGACY_LAUNCHER_FILE: &str = "musicmt-launcher.toml";
/// 曲库级配置文件名（位于 `<曲库>/.mt/`）。
pub const LIBRARY_CONFIG_FILE: &str = "config.toml";

// ============================================================
// 运行期合并视图
// ============================================================

/// 运行期有效配置 = app 级 + 曲库级的合并结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 当前曲库路径（app 级特例；app 配置文件中唯一与曲库相关的字段）。
    #[serde(default)]
    pub last_library_dir: Option<PathBuf>,
    pub paths: PathsConfig,
    pub netease: NeteaseConfig,
    pub ncm: NcmConfig,
    pub scanner: ScannerConfig,
    pub import: ImportConfig,
    pub library: LibraryConfig,
    pub server: ServerConfig,
    pub logging: LoggingConfig,
    /// 界面偏好（歌曲列表显示列等）。缺失时回退默认，向后兼容旧配置。
    #[serde(default)]
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathsConfig {
    #[serde(default)]
    pub app_data: PathBuf,
    /// 曲库根 = .mt 的父目录（扫描根）。运行时由 app_data.parent() 推导，不持久化绝对路径。
    #[serde(default)]
    pub download_root: PathBuf,
    #[serde(default)]
    pub download_dir: PathBuf,
    #[serde(default)]
    pub download_ing_dir: PathBuf,
    /// 已转换 ncm 的原始 .ncm 归档（= download_dir/ncm）。顶层 _ncm_raw 已废弃，统一到 download 体系。
    #[serde(default)]
    pub ncm_raw_dir: PathBuf,
    #[serde(default)]
    pub tools_dir: PathBuf,
}

/// 网易云音乐（本地导入源）。2026-10-29 起不再含登录/下载能力：
/// 本工具不联网、不登录；仅通过该目录把用户在官方 APP 内维护的曲库增量合并进来。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NeteaseConfig {
    /// 本地网易云音乐曲库目录（导入源）。指向用户在网易云官方 APP 内下载/维护的目录。
    /// 空 = 未设置；`从本地网易云音乐曲库更新` 按钮会提示先设置。
    #[serde(default)]
    pub library_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NcmConfig {
    /// NCM 解密工具路径（如 ncmdump.exe）。置空或指向不存在的文件 = 未配置。
    /// 唯一开关 = tool_ready()（tool_path 指向存在的文件），指定了工具即自动转换所有 .ncm。
    #[serde(default)]
    pub tool_path: PathBuf,
    #[serde(default = "default_command_template")]
    pub command_template: String,
}

/// NCM 命令模板默认值。
fn default_command_template() -> String {
    r#"{tool} "{input}""#.to_string()
}

impl Default for NcmConfig {
    fn default() -> Self {
        Self { tool_path: PathBuf::new(), command_template: default_command_template() }
    }
}

impl NcmConfig {
    /// NCM 转换的唯一开关：指定了工具（路径存在）即视为已配置，所有 .ncm 自动转换。
    pub fn tool_ready(&self) -> bool {
        self.tool_path.is_file()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    #[serde(default = "default_audio_meta")]
    pub audio_meta: String,
    #[serde(default = "default_match_strategy")]
    pub match_strategy: String,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            audio_meta: default_audio_meta(),
            match_strategy: default_match_strategy(),
        }
    }
}

/// 音频元信息抽取器默认值。
fn default_audio_meta() -> String {
    "lofty".to_string()
}

/// 扫描匹配策略默认值。
fn default_match_strategy() -> String {
    "clean_name_db".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportConfig {
    #[serde(default = "default_import_mode")]
    pub default_mode: String,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self { default_mode: default_import_mode() }
    }
}

/// 导入方式默认值（copy / move）。
fn default_import_mode() -> String {
    "copy".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryConfig {
    /// 文件结构方案（决定歌曲在曲库根下的归位方式）。
    /// artist_album | album | artist | flat（默认 artist_album）。
    #[serde(default = "default_file_scheme")]
    pub file_scheme: String,
    /// 曲库名称（用户自定义，便于在多曲库 / 多用户场景下区分）。
    /// 持久化于本曲库 .mt/config.toml；缺省时回退到曲库目录名。
    /// `#[serde(default)]` 保证旧配置（无该字段）仍能正常加载。
    #[serde(default)]
    pub name: String,
}

/// 文件结构方案默认值。
fn default_file_scheme() -> String {
    "artist_album".to_string()
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self { file_scheme: default_file_scheme(), name: String::new() }
    }
}

impl LibraryConfig {
    /// 合法方案集合（docs/05 §4a）。
    pub fn is_valid_scheme(scheme: &str) -> bool {
        matches!(scheme, "artist_album" | "album" | "artist" | "flat")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_server_host")]
    pub host: String,
    #[serde(default)]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { host: default_server_host(), port: 0 }
    }
}

/// 本地服务监听地址默认值（仅监听回环，避免暴露到局域网）。
fn default_server_host() -> String {
    "127.0.0.1".to_string()
}

/// 日志配置（app 级）。日志文件路径由曲库位置推导（`<曲库>/.mt/app.log`），故不持久化。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self { level: default_log_level() }
    }
}

/// 日志级别默认值。
fn default_log_level() -> String {
    "info".to_string()
}

/// 布局风格：**横向(1)** = 原布局（歌曲列表在左，曲目详情在右侧独立详情栏）；
/// **纵向(2)** = 新布局（曲目详情内联到歌曲列表上方，把宽度让给列表）。
pub const LAYOUT_STYLE_HORIZONTAL: u8 = 1;
pub const LAYOUT_STYLE_VERTICAL: u8 = 2;

/// 四个导航视图（对应左侧栏四个入口）。
pub const VIEW_KEYS: [&str; 4] = ["library", "playlists", "albums", "artists"];

/// 界面偏好配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    /// 歌曲列表显示列：有序的**可见**列 key 列表（顺序即显示顺序）。
    /// 可选 key：name(曲名) / artist(艺人) / album(专辑) / quality_tier(音质)
    ///          / duration_ms(时长) / file_size(文件大小)
    ///          / imported_at(导入时间) / created_at(创建时间)。
    /// 缺失时回退 `default_track_columns()`：导入时间默认显示，**创建时间默认隐藏**。
    #[serde(default = "default_track_columns")]
    pub track_columns: Vec<String>,
    /// 是否曾由「设置 → 列表 → 应用显示设置」显式保存过显示列。
    /// 用于区分「旧版本从未保存过（此时要按新默认规范迁移：补导入时间、不补创建时间）」与
    /// 「用户刻意把所有时间列都关掉」——两者在 `track_columns` 的值上可能完全一样，
    /// 必须靠这个标志判别，否则会反复把用户主动隐藏的导入时间又加回来。
    #[serde(default)]
    pub track_columns_saved: bool,
    /// 「全部曲库」页布局风格（1 = 横向 / 2 = 纵向）。默认 **横向**（原布局）。
    #[serde(default = "default_layout_library")]
    pub layout_library: u8,
    /// 「播放列表」页布局风格（1 = 横向 / 2 = 纵向）。默认 **纵向**（详情内联到列表上方）。
    #[serde(default = "default_layout_browse")]
    pub layout_playlists: u8,
    /// 「专辑」页布局风格（1 = 横向 / 2 = 纵向）。默认 **纵向**。
    #[serde(default = "default_layout_browse")]
    pub layout_albums: u8,
    /// 「歌手」页布局风格（1 = 横向 / 2 = 纵向）。默认 **纵向**。
    #[serde(default = "default_layout_browse")]
    pub layout_artists: u8,
    /// 「曲名」列宽度占比（%）。八列占比之和恒为 100。
    #[serde(default = "default_width_name")]
    pub width_name: f64,
    /// 「艺人」列宽度占比（%）。
    #[serde(default = "default_width_artist")]
    pub width_artist: f64,
    /// 「专辑」列宽度占比（%）。
    #[serde(default = "default_width_album")]
    pub width_album: f64,
    /// 「音质」列宽度占比（%）。
    #[serde(default = "default_width_quality_tier")]
    pub width_quality_tier: f64,
    /// 「时长」列宽度占比（%）。
    #[serde(default = "default_width_duration_ms")]
    pub width_duration_ms: f64,
    /// 「文件大小」列宽度占比（%）。
    #[serde(default = "default_width_file_size")]
    pub width_file_size: f64,
    /// 「导入时间」列宽度占比（%）。
    #[serde(default = "default_width_imported_at")]
    pub width_imported_at: f64,
    /// 「创建时间」列宽度占比（%）。
    #[serde(default = "default_width_created_at")]
    pub width_created_at: f64,
    // ---- 外观 / 主题方案（app 级偏好，与布局、列宽同落在 [ui] 段）----
    // 说明：自定义皮肤的图片本体是 base64 data URL（可达数百 KB～数 MB），**不写入 TOML**，
    // 仍由前端 localStorage 兜底；配置文件只存能复现「主题方案」的轻量字段。
    // 一律用 String（空串 = 未设置），避免 Option::None 在 TOML 序列化时被跳过，
    // 导致「清除皮肤 / 恢复默认」时旧值无法被就地覆盖。
    /// 主题模式：light(浅色) / system(跟随系统) / dark(暗色)。默认 system。
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 主题色（#RRGGBB）；空串 = 用明暗主题的默认派生色。
    #[serde(default)]
    pub accent: String,
    /// 皮肤类型：""(无皮肤) / preset(预设渐变) / custom(自定义图片)。
    #[serde(default)]
    pub skin_type: String,
    /// 预设皮肤 id（仅 skin_type=preset 有效，如 aurora / sakura / mint / purple / ocean）。
    #[serde(default)]
    pub skin_preset: String,
    /// 皮肤派生强调色（#RRGGBB）；皮肤激活时优先于 accent。
    #[serde(default)]
    pub skin_accent: String,
    /// 是否曾由前端显式保存过外观方案。
    /// 用于区分「旧版本从未写入外观（此时应把 localStorage 里的老偏好迁移进配置）」与
    /// 「用户显式把主题设成默认值」——两者在字段值上完全一样，必须靠这个标志判别。
    #[serde(default)]
    pub appearance_saved: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            track_columns: default_track_columns(),
            track_columns_saved: false,
            layout_library: default_layout_library(),
            layout_playlists: default_layout_browse(),
            layout_albums: default_layout_browse(),
            layout_artists: default_layout_browse(),
            width_name: default_width_name(),
            width_artist: default_width_artist(),
            width_album: default_width_album(),
            width_quality_tier: default_width_quality_tier(),
            width_duration_ms: default_width_duration_ms(),
            width_file_size: default_width_file_size(),
            width_imported_at: default_width_imported_at(),
            width_created_at: default_width_created_at(),
            theme: default_theme(),
            accent: String::new(),
            skin_type: String::new(),
            skin_preset: String::new(),
            skin_accent: String::new(),
            appearance_saved: false,
        }
    }
}

impl UiConfig {
    /// 取某个视图的布局风格（未知视图回退横向）。
    pub fn layout_for(&self, view: &str) -> u8 {
        match view {
            "library" => self.layout_library,
            "playlists" => self.layout_playlists,
            "albums" => self.layout_albums,
            "artists" => self.layout_artists,
            _ => LAYOUT_STYLE_HORIZONTAL,
        }
    }

    /// 设置某个视图的布局风格（非法值忽略）。
    pub fn set_layout(&mut self, view: &str, style: u8) -> bool {
        if style != LAYOUT_STYLE_HORIZONTAL && style != LAYOUT_STYLE_VERTICAL {
            return false;
        }
        match view {
            "library" => self.layout_library = style,
            "playlists" => self.layout_playlists = style,
            "albums" => self.layout_albums = style,
            "artists" => self.layout_artists = style,
            _ => return false,
        }
        true
    }

    /// 取某个列 key 的宽度占比（%）；未知列返回 0。
    pub fn width_for(&self, key: &str) -> f64 {
        match key {
            "name" => self.width_name,
            "artist" => self.width_artist,
            "album" => self.width_album,
            "quality_tier" => self.width_quality_tier,
            "duration_ms" => self.width_duration_ms,
            "file_size" => self.width_file_size,
            "imported_at" => self.width_imported_at,
            "created_at" => self.width_created_at,
            _ => 0.0,
        }
    }

    /// 设置某个列 key 的宽度占比（%）；非法值忽略。
    pub fn set_width(&mut self, key: &str, value: f64) -> bool {
        if !value.is_finite() || value <= 0.0 {
            return false;
        }
        match key {
            "name" => self.width_name = value,
            "artist" => self.width_artist = value,
            "album" => self.width_album = value,
            "quality_tier" => self.width_quality_tier = value,
            "duration_ms" => self.width_duration_ms = value,
            "file_size" => self.width_file_size = value,
            "imported_at" => self.width_imported_at = value,
            "created_at" => self.width_created_at = value,
            _ => return false,
        }
        true
    }

    /// 全部列的宽度占比（key → %），供接口返回。
    pub fn widths_map(&self) -> std::collections::BTreeMap<String, f64> {
        let mut m = std::collections::BTreeMap::new();
        for k in all_track_column_keys() {
            let w = self.width_for(&k);
            m.insert(k, w);
        }
        m
    }

    /// 四个视图的布局风格（view → 1=横向 / 2=纵向），供接口返回。
    pub fn layout_styles_map(&self) -> std::collections::BTreeMap<String, u8> {
        let mut m = std::collections::BTreeMap::new();
        for v in VIEW_KEYS {
            m.insert(v.to_string(), self.layout_for(v));
        }
        m
    }

    /// 归一化：布局风格限定为 1（横向）/ 2（纵向）；列宽占比限定在 [MIN, MAX] 且总和归一到 100。
    ///
    /// 用户手改配置文件可能写出越界值（如占比和为 0 或 500），这里统一修正，
    /// 保证前端拿到的比例始终可用。
    pub fn sanitize(&mut self) {
        // 显示列：先过滤非法 / 重复的 key，空列表回退默认。
        let allowed = all_track_column_keys();
        let mut seen = std::collections::HashSet::new();
        self.track_columns
            .retain(|k| allowed.contains(k) && seen.insert(k.clone()));
        if self.track_columns.is_empty() {
            self.track_columns = default_track_columns();
        }
        // 旧配置迁移：track_columns_saved=false 表示「从未在设置面板里显式保存过显示项」。
        //   - 补上默认显示的「导入时间」（保留用户原有的列顺序与子集）；
        //   - 不补「创建时间」——它默认隐藏，只能由用户勾选后经 /api/columns 打开。
        // 显式保存过之后就完全尊重用户选择（包括把所有时间列都关掉），不再自动回填。
        if !self.track_columns_saved {
            self.track_columns.retain(|k| k != "created_at");
            if !self.track_columns.iter().any(|k| k == "imported_at") {
                self.track_columns.push("imported_at".into());
            }
        }
        for v in VIEW_KEYS {
            let s = self.layout_for(v);
            if s != LAYOUT_STYLE_HORIZONTAL && s != LAYOUT_STYLE_VERTICAL {
                self.set_layout(v, LAYOUT_STYLE_HORIZONTAL);
            }
        }
        let keys = all_track_column_keys();
        for k in &keys {
            let v = self.width_for(k);
            if !v.is_finite() || v < MIN_COLUMN_WIDTH_PCT {
                self.set_width(k, default_width_for(k));
            }
        }
        let sum: f64 = keys.iter().map(|k| self.width_for(k)).sum();
        if sum > 0.0 && (sum - 100.0).abs() > 0.01 {
            let scale = 100.0 / sum;
            for k in &keys {
                let v = self.width_for(k) * scale;
                self.set_width(k, v);
            }
        }
        // 按比例缩放后，原本贴近下限的列可能又被压到下限以下（列数越多越明显）。
        // 二次校正：把不足下限的列抬到下限，再从其余列的「富余量」里按占比扣回，
        // 保证「每列 ≥ 下限」与「总和 = 100」同时成立。
        let min = MIN_COLUMN_WIDTH_PCT;
        let mut deficit = 0.0;
        for k in &keys {
            let v = self.width_for(k);
            if v < min {
                deficit += min - v;
                self.set_width(k, min);
            }
        }
        if deficit > 0.0 {
            let headroom: f64 = keys
                .iter()
                .map(|k| (self.width_for(k) - min).max(0.0))
                .sum();
            // 合法配置恒有 100 > 列数 × 下限，headroom - deficit > 0。
            if headroom > 0.0 {
                let factor = ((headroom - deficit) / headroom).max(0.0);
                for k in &keys {
                    let extra = (self.width_for(k) - min).max(0.0) * factor;
                    self.set_width(k, min + extra);
                }
            }
        }
        // 外观偏好：非法主题回退 system；颜色必须是 #RGB / #RRGGBB；
        // 皮肤类型只认 ""/preset/custom，且非预设皮肤不留 preset id、无皮肤不留派生色。
        if !THEME_MODES.contains(&self.theme.as_str()) {
            self.theme = default_theme();
        }
        if !self.accent.is_empty() && !is_hex_color(&self.accent) {
            self.accent.clear();
        }
        if !self.skin_accent.is_empty() && !is_hex_color(&self.skin_accent) {
            self.skin_accent.clear();
        }
        if !matches!(self.skin_type.as_str(), "" | "preset" | "custom") {
            self.skin_type.clear();
        }
        if self.skin_type != "preset" {
            self.skin_preset.clear();
        }
        if self.skin_type.is_empty() {
            self.skin_accent.clear();
        }
    }
}

/// 合法的主题模式（与前端 #ap-theme-seg 的三个分段按钮一致）。
pub const THEME_MODES: [&str; 3] = ["light", "system", "dark"];

fn default_theme() -> String {
    "system".to_string()
}

/// 是否为 #RGB / #RRGGBB 形式的颜色（配置文件里手写的颜色必须过这道校验）。
pub fn is_hex_color(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 4 && b.len() != 7 {
        return false;
    }
    b[0] == b'#' && b[1..].iter().all(|c| c.is_ascii_hexdigit())
}

/// 列宽占比下限（%）：防止某列被拖到几乎不可见。
pub const MIN_COLUMN_WIDTH_PCT: f64 = 4.0;

/// 「全部曲库」页默认布局：横向（原布局）。
fn default_layout_library() -> u8 {
    LAYOUT_STYLE_HORIZONTAL
}

/// 「播放列表 / 专辑 / 歌手」页默认布局：纵向（详情内联到列表上方）。
fn default_layout_browse() -> u8 {
    LAYOUT_STYLE_VERTICAL
}

fn default_width_name() -> f64 {
    24.0
}
fn default_width_artist() -> f64 {
    16.0
}
fn default_width_album() -> f64 {
    18.0
}
fn default_width_quality_tier() -> f64 {
    6.0
}
fn default_width_duration_ms() -> f64 {
    6.0
}
fn default_width_file_size() -> f64 {
    8.0
}
fn default_width_imported_at() -> f64 {
    11.0
}
fn default_width_created_at() -> f64 {
    11.0
}

/// 某列的默认宽度占比（%）。
fn default_width_for(key: &str) -> f64 {
    match key {
        "name" => default_width_name(),
        "artist" => default_width_artist(),
        "album" => default_width_album(),
        "quality_tier" => default_width_quality_tier(),
        "duration_ms" => default_width_duration_ms(),
        "file_size" => default_width_file_size(),
        "imported_at" => default_width_imported_at(),
        "created_at" => default_width_created_at(),
        _ => 10.0,
    }
}

/// 歌曲列表可用的全部列 key（顺序即默认显示顺序）。
pub fn all_track_column_keys() -> Vec<String> {
    vec![
        "name".into(),
        "artist".into(),
        "album".into(),
        "quality_tier".into(),
        "duration_ms".into(),
        "file_size".into(),
        "imported_at".into(),
        "created_at".into(),
    ]
}

/// 默认可见列（按 all_track_column_keys 顺序）。
///
/// 「导入时间」默认显示；「创建时间」**默认不显示**——它仍是一个合法列，
/// 只能由「设置 → 列表 → 歌曲列表显示项」勾选后经 `/api/columns` 显式打开。
/// 这样既保留数据完整性，又不让默认曲目列表被额外的时间列挤占宽度。
fn default_track_columns() -> Vec<String> {
    all_track_column_keys()
        .into_iter()
        .filter(|k| k != "created_at")
        .collect()
}

// ============================================================
// 两份持久化文件
// ============================================================

/// 配置文件版本元信息（写入配置文件顶层的 `[meta]` 段）。
///
/// 两个字段的含义：
/// - `config_version`：**生成该配置文件时的构建版本号**（写入时取当前 `APP_VERSION`）；
/// - `min_compatible_version`：**可正常解析该配置的最低应用版本号**（当前为 `MIN_COMPATIBLE_VERSION`）。
///
/// 读取配置时据此判断兼容性（见 `assess_compat`）：兼容则**沿用现有内容**，仅按需更新这两个字段；
/// 不兼容才允许重建，并必须说明冲突原因（见 `ConfigCompat::Conflict`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ConfigMeta {
    /// 生成该配置文件的构建版本号。
    #[serde(default)]
    pub config_version: String,
    /// 能够正常解析该配置文件的最低应用版本号。
    #[serde(default)]
    pub min_compatible_version: String,
}

impl ConfigMeta {
    /// 当前构建版本对应的元信息（写盘时使用）。
    pub fn current() -> Self {
        ConfigMeta {
            config_version: crate::app::APP_VERSION.to_string(),
            min_compatible_version: crate::app::MIN_COMPATIBLE_VERSION.to_string(),
        }
    }

    /// 是否为「无版本信息」的旧配置（缺 `[meta]` 段或字段为空）。
    pub fn is_empty(&self) -> bool {
        self.config_version.is_empty() && self.min_compatible_version.is_empty()
    }
}

/// 配置文件与当前程序的**兼容判定结果**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigCompat {
    /// 兼容：沿用现有配置内容，仅按需更新版本信息，**不删除、不重写**。
    Compatible,
    /// 旧格式（无 `[meta]` 版本信息）：内容沿用，并补写版本信息。
    Legacy,
    /// 逻辑冲突（结构或字段不兼容）：允许重建配置，`reason` 明确说明冲突原因。
    Conflict { reason: String },
}

impl ConfigCompat {
    /// 是否需要重建配置（仅冲突时为真）。
    pub fn is_conflict(&self) -> bool {
        matches!(self, ConfigCompat::Conflict { .. })
    }

    /// 冲突原因（非冲突时为空串）。
    pub fn reason(&self) -> &str {
        match self {
            ConfigCompat::Conflict { reason } => reason,
            _ => "",
        }
    }

    /// 供接口/日志使用的短标签。
    pub fn label(&self) -> &'static str {
        match self {
            ConfigCompat::Compatible => "compatible",
            ConfigCompat::Legacy => "legacy",
            ConfigCompat::Conflict { .. } => "conflict",
        }
    }
}

/// **app 级配置**：存放到 app 自身文件夹（exe 目录）的 `kqk-app.toml`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// 当前曲库路径（app 级特例）。
    #[serde(default)]
    pub last_library_dir: Option<PathBuf>,
    /// 版本元信息（`[meta]` 段）。旧配置没有该段时回退为空（判为 Legacy，内容仍沿用）。
    #[serde(default)]
    pub meta: ConfigMeta,
    #[serde(default)]
    pub netease: NeteaseConfig,
    #[serde(default)]
    pub ncm: NcmConfig,
    #[serde(default)]
    pub scanner: ScannerConfig,
    #[serde(default)]
    pub import: ImportConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

/// **曲库级配置**：存放到 `<曲库>/.mt/config.toml`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySettings {
    #[serde(default)]
    pub paths: PathsConfig,
    #[serde(default)]
    pub library: LibraryConfig,
    /// 版本元信息（`[meta]` 段）。
    #[serde(default)]
    pub meta: ConfigMeta,
}

/// 配置文件版本状态（供接口 / 界面展示版本信息与冲突原因）。
#[derive(Debug, Clone, Default)]
pub struct ConfigStatus {
    /// app 级配置（`kqk-app.toml`）记录的生成版本号。
    pub app_config_version: String,
    /// app 级配置要求的最低兼容版本号。
    pub app_min_compatible_version: String,
    /// app 级配置兼容判定：`compatible` / `legacy` / `conflict`。
    pub app_compat: String,
    /// app 级配置冲突原因（无冲突为空串）。
    pub app_conflict_reason: String,
    /// 曲库级配置（`.mt/config.toml`）记录的生成版本号。
    pub library_config_version: String,
    /// 曲库级配置要求的最低兼容版本号。
    pub library_min_compatible_version: String,
    /// 曲库级配置兼容判定。
    pub library_compat: String,
    /// 曲库级配置冲突原因（无冲突为空串）。
    pub library_conflict_reason: String,
}

impl ConfigStatus {
    /// 由启动加载结果构造。
    pub fn from_loaded(l: &LoadedConfig) -> Self {
        ConfigStatus {
            app_config_version: l.app_meta.config_version.clone(),
            app_min_compatible_version: l.app_meta.min_compatible_version.clone(),
            app_compat: l.app_compat.label().to_string(),
            app_conflict_reason: l.app_compat.reason().to_string(),
            library_config_version: l.library_meta.config_version.clone(),
            library_min_compatible_version: l.library_meta.min_compatible_version.clone(),
            library_compat: l.library_compat.label().to_string(),
            library_conflict_reason: l.library_compat.reason().to_string(),
        }
    }
}

/// 启动时加载到的两份配置及其路径。
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    /// 运行期有效配置（合并视图）。
    pub cfg: Config,
    /// app 配置文件路径（`kqk-app.toml`）。
    pub app_path: PathBuf,
    /// 曲库配置文件路径（`<曲库>/.mt/config.toml`；未配置曲库时为 guest 路径）。
    pub library_path: PathBuf,
    /// 当前曲库是否已初始化。
    pub library_configured: bool,
    /// app 级配置的版本元信息（磁盘上实际记录的）。
    pub app_meta: ConfigMeta,
    /// app 级配置的兼容判定结果。
    pub app_compat: ConfigCompat,
    /// 曲库级配置的版本元信息（磁盘上实际记录的）。
    pub library_meta: ConfigMeta,
    /// 曲库级配置的兼容判定结果。
    pub library_compat: ConfigCompat,
}

// ============================================================
// 默认值
// ============================================================

/// app 级默认设置。
pub fn default_app_settings() -> AppSettings {
    AppSettings {
        last_library_dir: None,
        meta: ConfigMeta::current(),
        netease: NeteaseConfig {
            library_dir: PathBuf::new(), // 未设置（向导或设置页指定后才写入）
        },
        ncm: NcmConfig {
            tool_path: PathBuf::new(), // 未配置（指定工具后写入，含 .mt/tools 内副本路径）
            command_template: r#"{tool} "{input}""#.into(),
        },
        scanner: ScannerConfig {
            audio_meta: "lofty".into(),
            match_strategy: "clean_name_db".into(),
        },
        import: ImportConfig {
            default_mode: "copy".into(),
        },
        server: ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        logging: LoggingConfig {
            level: "info".into(),
        },
        ui: UiConfig::default(),
    }
}

/// 曲库级默认设置（路径相对曲库根）。
pub fn default_library_settings(library_dir: &Path) -> LibrarySettings {
    let mt = library_dir.join(".mt");
    LibrarySettings {
        meta: ConfigMeta::current(),
        paths: PathsConfig {
            app_data: mt.clone(),
            download_root: library_dir.to_path_buf(),
            download_dir: library_dir.join("download"),
            download_ing_dir: library_dir.join("download").join("ing"),
            ncm_raw_dir: library_dir.join("download").join("ncm"),
            tools_dir: mt.join("tools"),
        },
        library: LibraryConfig {
            file_scheme: "artist_album".into(),
            name: String::new(),
        },
    }
}

/// 默认配置（与 docs/03 §3 对应）。仅用于 guest 态（未指定曲库）承载前端。
pub fn default_config() -> Config {
    let app = default_app_settings();
    let lib = default_library_settings(Path::new("./musiclibrary"));
    Config::merge(&app, &lib)
}

// ============================================================
// 路径解析
// ============================================================

/// app 配置文件路径：exe 所在目录下的 `kqk-app.toml`（不依赖运行时 CWD）。
pub fn app_config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(APP_CONFIG_FILE)))
        .unwrap_or_else(|| PathBuf::from(APP_CONFIG_FILE))
}

/// 旧版 launcher 文件路径（仅用于迁移）。
fn legacy_launcher_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(LEGACY_LAUNCHER_FILE)))
        .unwrap_or_else(|| PathBuf::from(LEGACY_LAUNCHER_FILE))
}

/// 曲库配置文件路径：`<曲库>/.mt/config.toml`。
pub fn library_config_path(library_dir: &Path) -> PathBuf {
    library_dir.join(".mt").join(LIBRARY_CONFIG_FILE)
}

/// app 自身文件夹（exe 所在目录）。
pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

// ============================================================
// 合并 / 拆分
// ============================================================

impl Config {
    /// 由 app 级 + 曲库级两份配置合并出运行期有效配置。
    pub fn merge(app: &AppSettings, lib: &LibrarySettings) -> Config {
        // 界面偏好统一归一化（布局风格限定 1/2、列宽占比限定范围且总和为 100），
        // 防止用户手改配置写出越界值导致前端渲染异常。
        let mut ui = app.ui.clone();
        ui.sanitize();
        Config {
            last_library_dir: app.last_library_dir.clone(),
            paths: lib.paths.clone(),
            netease: app.netease.clone(),
            ncm: app.ncm.clone(),
            scanner: app.scanner.clone(),
            import: app.import.clone(),
            library: lib.library.clone(),
            server: app.server.clone(),
            logging: app.logging.clone(),
            ui,
        }
    }

    /// 拆分为可分别持久化的两份（app 级 / 曲库级）。
    ///
    /// 注：`[meta]` 版本信息不由运行期视图承载（写盘时统一盖当前构建版本），
    /// 故此处产出空 `ConfigMeta`，由 `save_app` / `save_library` 填充。
    pub fn split(&self) -> (AppSettings, LibrarySettings) {
        (
            AppSettings {
                last_library_dir: self.last_library_dir.clone(),
                meta: ConfigMeta::default(),
                netease: self.netease.clone(),
                ncm: self.ncm.clone(),
                scanner: self.scanner.clone(),
                import: self.import.clone(),
                server: self.server.clone(),
                logging: self.logging.clone(),
                ui: self.ui.clone(),
            },
            LibrarySettings {
                paths: self.paths.clone(),
                library: self.library.clone(),
                meta: ConfigMeta::default(),
            },
        )
    }

    /// 日志文件路径：由曲库位置推导（`<曲库>/.mt/app.log`），不持久化绝对路径。
    pub fn log_path(&self) -> PathBuf {
        self.paths.app_data.join("app.log")
    }

    /// 确保运行时目录（app_data / download_root / download 体系 / ncm 归档）存在。
    pub fn ensure_dirs(&self) -> Result<()> {
        let pending = self.paths.pending_ncm_dir();
        for d in [&self.paths.app_data,
                  &self.paths.download_root,
                  &self.paths.download_dir,
                  &self.paths.download_ing_dir,
                  &self.paths.ncm_raw_dir,
                  &pending,
                  &self.paths.tools_dir] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    /// 保存 **app 级**配置到 app 自身文件夹（含当前曲库路径）。
    ///
    /// 写入时自动盖上当前构建版本的 `[meta]` 段（生成版本 + 最低兼容版本）。
    pub fn save_app(&self, path: &Path) -> Result<()> {
        let (mut app, _) = self.split();
        app.meta = ConfigMeta::current();
        write_toml(path, &app)
    }

    /// 保存 **曲库级**配置到 `<曲库>/.mt/config.toml`。
    ///
    /// 写入时自动盖上当前构建版本的 `[meta]` 段（生成版本 + 最低兼容版本）。
    pub fn save_library(&self, path: &Path) -> Result<()> {
        let (_, mut lib) = self.split();
        lib.meta = ConfigMeta::current();
        write_toml(path, &lib)
    }
}

/// 通用 TOML 写盘（自动建父目录）。
fn write_toml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(value)?;
    std::fs::write(path, text)?;
    Ok(())
}

// ============================================================
// 版本兼容判定与「保留式」配置维护
// ============================================================
//
// 设计目标（用户需求 2026-11-04）：
//   ① 既有配置文件**不得**在每次构建/启动时被删除或覆盖——只要新旧版本之间
//      不存在逻辑冲突，就必须**保留现有配置文件及其内容**；
//   ② 配置文件中记录两个版本号：生成时的构建版本 + 可正常运行的最低版本；
//   ③ 版本兼容时**跳过删除与重写**，直接沿用并按需更新版本信息；
//      出现逻辑冲突（结构或字段不兼容）时才允许替换，并**明确说明冲突原因**。
//
// 实现要点：
//   - 兼容性判定见 `assess_compat`；
//   - 版本信息用 `inject_meta` / `update_meta_in_place` **就地更新**，
//     只改 `[meta]` 段，其余内容（注释、未知字段、键序）逐行原样保留；
//   - 冲突时先 `backup_conflict` 备份原文件，再重建，并保留冲突原因供界面/日志展示。

/// 判定配置文件的版本兼容性。
///
/// 规则（docs/03 §3.4）：
/// 1. 文件解析失败 → **冲突**（结构不兼容）；
/// 2. 无 `[meta]` 版本信息 → **旧格式**：内容沿用，仅补写版本信息；
/// 3. `min_compatible_version` 高于当前程序版本 → **冲突**（当前程序过旧，读不懂）；
/// 4. `config_version` 高于当前程序版本 → **冲突**（配置由更高版本生成，结构可能已变）；
/// 5. 其余 → **兼容**：沿用现有内容，仅按需更新版本信息。
pub fn assess_compat(meta: &ConfigMeta, parse_error: Option<&str>) -> ConfigCompat {
    if let Some(e) = parse_error {
        return ConfigCompat::Conflict {
            reason: format!("配置文件解析失败（结构或字段不兼容）：{e}"),
        };
    }
    if meta.is_empty() {
        return ConfigCompat::Legacy;
    }
    let cur = crate::app::APP_VERSION;
    if !meta.min_compatible_version.is_empty()
        && crate::app::version_cmp(&meta.min_compatible_version, cur) == std::cmp::Ordering::Greater
    {
        return ConfigCompat::Conflict {
            reason: format!(
                "配置要求的最低兼容版本 v{} 高于当前程序版本 v{}，当前程序过旧，无法正确解析该配置",
                meta.min_compatible_version, cur
            ),
        };
    }
    if !meta.config_version.is_empty()
        && crate::app::version_cmp(&meta.config_version, cur) == std::cmp::Ordering::Greater
    {
        return ConfigCompat::Conflict {
            reason: format!(
                "配置由更高版本生成（配置 v{}，当前程序 v{}），结构可能已不兼容",
                meta.config_version, cur
            ),
        };
    }
    ConfigCompat::Compatible
}

/// 在 TOML 文本中就地写入/更新 `[meta]` 段的两个版本字段。
///
/// **只改 `[meta]` 段**：其余内容逐行原样保留（含注释、未知字段、键序），
/// 因此可安全地对既有配置「按需更新版本信息」而不破坏用户内容。
/// 若原文没有 `[meta]` 段，则在文件末尾追加一段。
pub fn inject_meta(text: &str, meta: &ConfigMeta) -> String {
    let cv = format!("config_version = \"{}\"", meta.config_version);
    let mv = format!("min_compatible_version = \"{}\"", meta.min_compatible_version);
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    let mut i = 0usize;
    let mut found = false;
    while i < lines.len() {
        let line = lines[i];
        if line.trim() == "[meta]" {
            found = true;
            out.push(line.to_string());
            i += 1;
            // 复制 [meta] 段内的其余行，同时替换/补齐两个版本键。
            let mut wrote_cv = false;
            let mut wrote_mv = false;
            while i < lines.len() {
                let l = lines[i];
                if l.trim_start().starts_with('[') {
                    break; // 下一个表头 → [meta] 段结束
                }
                let key = l.split('=').next().unwrap_or("").trim();
                if key == "config_version" {
                    out.push(cv.clone());
                    wrote_cv = true;
                } else if key == "min_compatible_version" {
                    out.push(mv.clone());
                    wrote_mv = true;
                } else {
                    out.push(l.to_string());
                }
                i += 1;
            }
            if !wrote_cv {
                out.push(cv.clone());
            }
            if !wrote_mv {
                out.push(mv.clone());
            }
            continue;
        }
        out.push(line.to_string());
        i += 1;
    }
    if !found {
        if !out.is_empty() && !out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            out.push(String::new());
        }
        out.push("[meta]".to_string());
        out.push(cv);
        out.push(mv);
    }
    let mut s = out.join("\n");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// 就地更新配置文件中的 `[meta]` 版本信息（**其余内容原样保留**）。
pub fn update_meta_in_place(path: &Path, meta: &ConfigMeta) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let new_text = inject_meta(&text, meta);
    if new_text == text {
        return Ok(()); // 无变化 → 不触碰文件（连时间戳都不动）
    }
    std::fs::write(path, new_text)?;
    Ok(())
}

/// `[ui]` 段的字段顺序（新增键按此顺序追加，保证输出稳定可读）。
pub const UI_FIELD_ORDER: [&str; 20] = [
    "track_columns",
    "track_columns_saved",
    "layout_library",
    "layout_playlists",
    "layout_albums",
    "layout_artists",
    "width_name",
    "width_artist",
    "width_album",
    "width_quality_tier",
    "width_duration_ms",
    "width_file_size",
    "width_imported_at",
    "width_created_at",
    "theme",
    "accent",
    "skin_type",
    "skin_preset",
    "skin_accent",
    "appearance_saved",
];

/// 计算一行 TOML 中未闭合的括号深度（忽略字符串字面量与行注释内的括号）。
///
/// 用于识别多行数组 / 内联表的值：深度 > 0 表示该值还没结束，
/// 后续行属于它的续行，不能当作独立的「键 = 值」处理。
fn toml_bracket_depth(line: &str) -> i32 {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut quote = '"';
    let mut escaped = false;
    for ch in line.chars() {
        if in_str {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                c if c == quote => in_str = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                in_str = true;
                quote = ch;
            }
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            '#' => break, // 行注释之后的都不是值
            _ => {}
        }
    }
    depth
}

/// 就地更新配置文件中的 `[ui]` 段（显示列 / 布局风格 / 列宽比例）。
///
/// **只改 `[ui]` 段内的已知键**：段内注释、未知键、既有键序原样保留
/// （与 `inject_meta` 同一策略——运行时改界面偏好不应破坏用户手写的内容）。
/// 缺失的键追加到段尾；若原文没有 `[ui]` 段，则在文件末尾追加整段。
pub fn upsert_ui_section(text: &str, ui: &UiConfig) -> String {
    // 序列化为「键 → 单行 TOML 值」，便于逐键替换。
    let table: toml::Table = toml::Value::try_from(ui)
        .ok()
        .and_then(|v| v.as_table().cloned())
        .unwrap_or_default();
    let mut new_line_for: std::collections::BTreeMap<String, String> = Default::default();
    for f in UI_FIELD_ORDER {
        if let Some(v) = table.get(f) {
            new_line_for.insert(f.to_string(), format!("{} = {}", f, v.to_string()));
        }
    }

    let mut out: Vec<String> = Vec::new();
    let mut lines = text.lines().peekable();
    let mut found = false;
    let mut written: std::collections::BTreeSet<String> = Default::default();
    while let Some(line) = lines.next() {
        if line.trim() == "[ui]" {
            found = true;
            out.push("[ui]".to_string());
            // 逐行处理 [ui] 段：已知键 → 替换为新值；注释 / 未知键 → 原样保留。
            while let Some(l) = lines.peek() {
                if l.trim_start().starts_with('[') {
                    break; // 下一个表头 → [ui] 段结束
                }
                let l = lines.next().unwrap();
                let key = l.split('=').next().unwrap_or("").trim().to_string();
                let known = !key.is_empty() && new_line_for.contains_key(&key);
                if known {
                    out.push(new_line_for[&key].clone());
                    written.insert(key);
                } else {
                    out.push(l.to_string());
                }
                // 多行值（多行数组 / 内联表）的续行不参与键匹配：
                // 已知键被替换时丢弃旧续行，未知键则原样保留。
                let mut depth = toml_bracket_depth(&l);
                while depth > 0 {
                    let nl = match lines.peek() {
                        Some(x) if !x.trim_start().starts_with('[') => lines.next().unwrap(),
                        _ => break,
                    };
                    if !known {
                        out.push(nl.to_string());
                    }
                    depth += toml_bracket_depth(&nl);
                }
            }
            // 补齐本段缺失的已知键（追加到段尾，保持既有键序不变）。
            for f in UI_FIELD_ORDER {
                if !written.contains(f) {
                    if let Some(line) = new_line_for.get(f) {
                        out.push(line.clone());
                    }
                }
            }
            continue;
        }
        out.push(line.to_string());
    }
    if found {
        // 已在 [ui] 段内补齐，无需额外处理。
    } else {
        if !out.is_empty() && !out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            out.push(String::new());
        }
        out.push("[ui]".to_string());
        for f in UI_FIELD_ORDER {
            if let Some(line) = new_line_for.get(f) {
                out.push(line.clone());
            }
        }
    }
    let mut s = out.join("\n");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// 把界面偏好写回 app 配置文件（就地更新 `[ui]` 段，并同步 `[meta]` 版本信息）。
pub fn save_ui_in_place(path: &Path, ui: &UiConfig) -> Result<()> {
    let mut normalized = ui.clone();
    normalized.sanitize();
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let new_text = inject_meta(&upsert_ui_section(&text, &normalized), &ConfigMeta::current());
    std::fs::write(path, new_text)?;
    Ok(())
}

/// 冲突时备份原配置文件，返回备份路径（保留用户内容，便于事后追溯）。
///
/// 备份名：`<原文件名>.conflict-<当前版本>.bak`，同版本重复冲突不覆盖上一份。
pub fn backup_conflict(path: &Path) -> Option<PathBuf> {
    let stem = path.file_name()?.to_string_lossy().to_string();
    let bak = path.with_file_name(format!(
        "{stem}.conflict-{}.bak",
        crate::app::APP_VERSION
    ));
    std::fs::copy(path, &bak).ok().map(|_| bak)
}

// ============================================================
// 加载
// ============================================================

/// 从任意 TOML 文本中**抢救**语义稳定的标量字段（路径类），用于配置冲突时的重建。
///
/// 抢救这些字段的理由：它们是跨版本稳定的路径字符串，即使配置结构发生变化，
/// 其含义也不会变。抢救后用户无需重新指定曲库 / NCM 工具 / 导入源，
/// 避免「每次构建都要重新指定」的困扰。语义可能变化的复杂结构则不抢救。
fn salvage_app_settings(text: &str) -> AppSettings {
    let mut app = default_app_settings();
    let Ok(v) = text.parse::<toml::Value>() else {
        return app;
    };
    if let Some(p) = v.get("last_library_dir").and_then(|x| x.as_str()) {
        if !p.is_empty() {
            app.last_library_dir = Some(PathBuf::from(p));
        }
    }
    if let Some(p) = v
        .get("netease")
        .and_then(|n| n.get("library_dir"))
        .and_then(|x| x.as_str())
    {
        if !p.is_empty() {
            app.netease.library_dir = PathBuf::from(p);
        }
    }
    if let Some(p) = v
        .get("ncm")
        .and_then(|n| n.get("tool_path"))
        .and_then(|x| x.as_str())
    {
        if !p.is_empty() {
            app.ncm.tool_path = PathBuf::from(p);
        }
    }
    app
}

/// 读取 app 级配置 + 兼容判定。返回 `(配置, 判定, 文件是否原本存在)`。
///
/// - 文件不存在 → 默认配置 + `Compatible`（首次生成，由调用方写盘）；
/// - 解析失败 / 版本冲突 → 默认配置 + `Conflict{reason}`（调用方备份后重建）；
/// - 解析成功 → **原样沿用** + `Compatible` / `Legacy`。
fn load_app_settings_with_compat(path: &Path) -> (AppSettings, ConfigCompat, bool) {
    if !path.exists() {
        return (default_app_settings(), ConfigCompat::Compatible, false);
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            return (
                default_app_settings(),
                ConfigCompat::Conflict { reason: format!("读取配置文件失败：{e}") },
                true,
            )
        }
    };
    match toml::from_str::<AppSettings>(&text) {
        Ok(s) => {
            let compat = assess_compat(&s.meta, None);
            if compat.is_conflict() {
                // 冲突：仍抢救路径类稳定字段，避免用户重新指定曲库。
                (salvage_app_settings(&text), compat, true)
            } else {
                (s, compat, true)
            }
        }
        Err(e) => (
            salvage_app_settings(&text),
            ConfigCompat::Conflict {
                reason: format!("配置文件解析失败（结构或字段不兼容）：{e}"),
            },
            true,
        ),
    }
}

/// 读取曲库级配置 + 兼容判定。返回 `(配置, 判定, 文件是否原本存在)`。
fn load_library_settings_with_compat(
    path: &Path,
    library_dir: &Path,
) -> (LibrarySettings, ConfigCompat, bool) {
    if !path.exists() {
        return (
            default_library_settings(library_dir),
            ConfigCompat::Compatible,
            false,
        );
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            return (
                default_library_settings(library_dir),
                ConfigCompat::Conflict { reason: format!("读取配置文件失败：{e}") },
                true,
            )
        }
    };
    match toml::from_str::<LibrarySettings>(&text) {
        Ok(s) => {
            let compat = assess_compat(&s.meta, None);
            if compat.is_conflict() {
                (default_library_settings(library_dir), compat, true)
            } else {
                (s, compat, true)
            }
        }
        Err(e) => {
            // 解析失败：尽量抢救 `[library]` 段（曲库名称 / 结构方案是用户可见的设置），
            // 其余（paths）按曲库目录重新推导。
            let mut lib = default_library_settings(library_dir);
            if let Ok(v) = text.parse::<toml::Value>() {
                if let Some(l) = v.get("library") {
                    if let Ok(parsed) = l.clone().try_into::<LibraryConfig>() {
                        lib.library = parsed;
                    }
                }
            }
            (
                lib,
                ConfigCompat::Conflict {
                    reason: format!("配置文件解析失败（结构或字段不兼容）：{e}"),
                },
                true,
            )
        }
    }
}

/// 读取 app 级配置（文件不存在 → 写入默认配置后返回）。
pub fn load_app_settings(path: &Path) -> Result<AppSettings> {
    if path.exists() {
        let text = std::fs::read_to_string(path)?;
        // 解析失败时回退默认（不让一个坏文件导致应用无法启动）。
        Ok(toml::from_str(&text).unwrap_or_else(|_| default_app_settings()))
    } else {
        let app = default_app_settings();
        write_toml(path, &app)?;
        Ok(app)
    }
}

/// 读取曲库级配置（文件不存在 → 写入默认配置后返回）。
pub fn load_library_settings(path: &Path) -> Result<LibrarySettings> {
    if path.exists() {
        let text = std::fs::read_to_string(path)?;
        // 旧版曲库配置含 app 级段落（netease/ncm/…），此处按未知字段忽略。
        Ok(toml::from_str(&text).unwrap_or_else(|_| {
            default_library_settings(path.parent().and_then(|p| p.parent()).unwrap_or(Path::new(".")))
        }))
    } else {
        let lib = default_library_settings(path.parent().and_then(|p| p.parent()).unwrap_or(Path::new(".")));
        write_toml(path, &lib)?;
        Ok(lib)
    }
}

/// 读取旧版 launcher 中的曲库路径（仅迁移用）。
fn load_legacy_last_library() -> Option<PathBuf> {
    let text = std::fs::read_to_string(legacy_launcher_path()).ok()?;
    let val: std::collections::HashMap<String, String> = toml::from_str(&text).ok()?;
    let dir = val.get("last_library_dir")?;
    let pb = PathBuf::from(dir);
    if pb.is_dir() {
        Some(pb)
    } else {
        None
    }
}

/// 旧版曲库配置（app 级与曲库级混在一份文件里）——迁移来源。
#[derive(Debug, Clone, Deserialize)]
struct LegacyFullConfig {
    #[serde(default)]
    netease: Option<NeteaseConfig>,
    #[serde(default)]
    ncm: Option<NcmConfig>,
    #[serde(default)]
    scanner: Option<ScannerConfig>,
    #[serde(default)]
    import: Option<ImportConfig>,
    #[serde(default)]
    server: Option<ServerConfig>,
    #[serde(default)]
    logging: Option<LoggingConfig>,
    #[serde(default)]
    ui: Option<UiConfig>,
}

/// 读取旧版曲库配置中的 app 级段落（迁移用）。
fn read_legacy_app_sections(library_dir: &Path) -> Option<LegacyFullConfig> {
    let p = library_config_path(library_dir);
    let text = std::fs::read_to_string(p).ok()?;
    toml::from_str(&text).ok()
}

/// 在 TOML 文本中就地写入/更新某个**顶层键**（键必须位于首个表头之前）。
///
/// 用于在不重写整份文件的前提下补写个别字段（如迁移来的 `last_library_dir`），
/// 保留其余内容（含注释与未知字段）原样。
pub fn upsert_top_level(text: &str, key: &str, value_literal: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut replaced = false;
    let mut inserted = false;
    for line in lines.iter() {
        let t = line.trim();
        // 遇到首个表头 → 顶层键区结束；若目标键还没写，就插在表头之前。
        if t.starts_with('[') && !inserted && !replaced {
            out.push(format!("{key} = {value_literal}"));
            inserted = true;
        }
        if !replaced && !t.starts_with('[') {
            let k = t.split('=').next().unwrap_or("").trim();
            if k == key {
                out.push(format!("{key} = {value_literal}"));
                replaced = true;
                continue;
            }
        }
        out.push(line.to_string());
    }
    if !replaced && !inserted {
        // 全文无表头 → 追加到末尾；否则已在表头前插入。
        out.push(format!("{key} = {value_literal}"));
    }
    let mut s = out.join("\n");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// 启动加载：读 app 配置 + 当前曲库配置，合并为运行期配置。
///
/// **配置保留策略（2026-11-04）**：
/// - 版本兼容（`Compatible`）→ **沿用现有配置文件及其内容**，仅当 `[meta]` 版本信息
///   需要更新时才**就地**改写该段，其余内容（注释/未知字段/键序）逐字保留；
/// - 旧格式无版本信息（`Legacy`）→ 同样沿用内容，仅补写 `[meta]` 段；
/// - 逻辑冲突（`Conflict`）→ 才允许替换：先把原文件备份为
///   `*.conflict-<版本>.bak`，再重建配置，并保留冲突原因供界面与日志展示。
///
/// 迁移策略（仅首次生成 app 配置时生效）：
/// 1. 旧 `musicmt-launcher.toml` 的 `last_library_dir` → app 配置的 `last_library_dir`；
/// 2. 旧曲库 `config.toml` 里混放的 app 级段落（NCM 工具、导入源、界面偏好等）→ app 配置。
pub fn load_startup() -> Result<LoadedConfig> {
    let app_path = app_config_path();
    let (mut app, app_compat, app_existed) = load_app_settings_with_compat(&app_path);

    // 迁移 1：旧 launcher → app 配置的当前曲库路径（已有值则不覆盖）。
    let had_last = app.last_library_dir.is_some();
    if !had_last {
        app.last_library_dir = load_legacy_last_library();
    }
    // 迁移 2：旧曲库配置里混放的 app 级段落 → app 配置（仅当 app 配置是首次生成的）。
    if !app_existed {
        if let Some(dir) = app.last_library_dir.clone() {
            if let Some(legacy) = read_legacy_app_sections(&dir) {
                if let Some(v) = legacy.netease {
                    app.netease = v;
                }
                if let Some(v) = legacy.ncm {
                    app.ncm = v;
                }
                if let Some(v) = legacy.scanner {
                    app.scanner = v;
                }
                if let Some(v) = legacy.import {
                    app.import = v;
                }
                if let Some(v) = legacy.server {
                    app.server = v;
                }
                if let Some(v) = legacy.logging {
                    app.logging = v;
                }
                if let Some(v) = legacy.ui {
                    app.ui = v;
                }
            }
        }
    }

    // ---- app 配置的版本信息落盘（保留式）----
    let cur_meta = ConfigMeta::current();
    let app_meta_on_disk = app.meta.clone();
    if !app_existed {
        // 首次生成：整份写出（含当前版本信息）。
        app.meta = cur_meta.clone();
        write_toml(&app_path, &app)?;
    } else {
        match &app_compat {
            ConfigCompat::Conflict { reason } => {
                // 逻辑冲突：备份原文件后重建，用户内容保留在 .bak 中。
                let bak = backup_conflict(&app_path);
                let kept = if app.last_library_dir.is_some() {
                    "（当前曲库路径等稳定字段已保留）"
                } else {
                    ""
                };
                eprintln!(
                    "[config] app 配置存在逻辑冲突，已备份并重建。原因：{reason}{kept}{}",
                    bak.map(|b| format!("（备份：{}）", b.display())).unwrap_or_default()
                );
                app.meta = cur_meta.clone();
                write_toml(&app_path, &app)?;
            }
            ConfigCompat::Legacy => {
                // 旧格式：内容沿用，仅补写版本信息。
                app.meta = cur_meta.clone();
                if let Err(e) = update_meta_in_place(&app_path, &app.meta) {
                    eprintln!("[config] 写入 app 配置版本信息失败：{e}");
                }
            }
            ConfigCompat::Compatible => {
                // 兼容：版本信息已是最新 → 完全不触碰文件（保留用户内容）。
                if app_meta_on_disk != cur_meta {
                    app.meta = cur_meta.clone();
                    if let Err(e) = update_meta_in_place(&app_path, &app.meta) {
                        eprintln!("[config] 更新 app 配置版本信息失败：{e}");
                    }
                }
            }
        }
    }
    // 迁移 1 补齐的曲库路径：就地写入顶层键，不重写整份文件。
    if app_existed && !had_last {
        if let Some(dir) = app.last_library_dir.clone() {
            let text = std::fs::read_to_string(&app_path).unwrap_or_default();
            let lit = toml::Value::String(dir.to_string_lossy().to_string()).to_string();
            let new_text = upsert_top_level(&text, "last_library_dir", &lit);
            if let Err(e) = std::fs::write(&app_path, new_text) {
                eprintln!("[config] 写入当前曲库路径失败：{e}");
            }
        }
    }

    // 当前曲库：路径存在且已初始化 .mt/config.toml 才算已配置。
    let current = app
        .last_library_dir
        .clone()
        .filter(|d| d.is_dir() && library_config_path(d).exists());

    match current {
        Some(dir) => {
            let lib_path = library_config_path(&dir);
            let (lib, library_compat, lib_existed) =
                load_library_settings_with_compat(&lib_path, &dir);
            let mut cfg = Config::merge(&app, &lib);
            cfg.last_library_dir = Some(dir.clone());

            // ---- 曲库配置的版本信息落盘（保留式，与 app 配置同一策略）----
            let lib_meta_on_disk = lib.meta.clone();
            if !lib_existed {
                cfg.save_library(&lib_path)?;
            } else {
                match &library_compat {
                    ConfigCompat::Conflict { reason } => {
                        let bak = backup_conflict(&lib_path);
                        eprintln!(
                            "[config] 曲库配置存在逻辑冲突，已备份并重建。原因：{reason}{}",
                            bak.map(|b| format!("（备份：{}）", b.display())).unwrap_or_default()
                        );
                        cfg.save_library(&lib_path)?;
                    }
                    ConfigCompat::Legacy => {
                        let mut m = lib_meta_on_disk.clone();
                        m.config_version = cur_meta.config_version.clone();
                        m.min_compatible_version = cur_meta.min_compatible_version.clone();
                        if let Err(e) = update_meta_in_place(&lib_path, &m) {
                            eprintln!("[config] 写入曲库配置版本信息失败：{e}");
                        }
                    }
                    ConfigCompat::Compatible => {
                        // 版本信息已是最新 → 不触碰文件，保留用户内容与注释。
                        if lib_meta_on_disk != cur_meta {
                            if let Err(e) = update_meta_in_place(&lib_path, &cur_meta) {
                                eprintln!("[config] 更新曲库配置版本信息失败：{e}");
                            }
                        }
                    }
                }
            }

            Ok(LoadedConfig {
                cfg,
                app_path,
                library_path: lib_path,
                library_configured: true,
                app_meta: app_meta_on_disk,
                app_compat,
                library_meta: lib_meta_on_disk,
                library_compat,
            })
        }
        None => {
            // Guest 态：未指定曲库，仅承载前端与 status。用 app 目录下的占位路径（不污染发布目录）。
            let guest = app_dir().join(".guest");
            let lib = LibrarySettings {
                paths: PathsConfig {
                    app_data: guest.clone(),
                    download_root: guest.clone(),
                    download_dir: guest.join("download"),
                    download_ing_dir: guest.join("download").join("ing"),
                    ncm_raw_dir: guest.join("download").join("ncm"),
                    tools_dir: guest.join("tools"),
                },
                library: LibraryConfig {
                    file_scheme: "artist_album".into(),
                    name: String::new(),
                },
                meta: cur_meta.clone(),
            };
            let mut cfg = Config::merge(&app, &lib);
            cfg.last_library_dir = None;
            Ok(LoadedConfig {
                cfg,
                app_path,
                library_path: guest.join(".mt").join(LIBRARY_CONFIG_FILE),
                library_configured: false,
                app_meta: app_meta_on_disk,
                app_compat,
                library_meta: cur_meta.clone(),
                library_compat: ConfigCompat::Compatible,
            })
        }
    }
}

/// 初始化（或加载已有）曲库：建 .mt、config.toml、library.db、README、download 体系。
/// 同时把该曲库记为「当前曲库」（写入 app 配置的 last_library_dir）。
/// 返回合并后的运行期配置（CLI / GUI 共用，docs/05 §4a/§5）。`name` 为空时回退到曲库目录名。
pub fn init_library(library_dir: &Path, scheme: &str, name: &str) -> Result<Config> {
    let mt = library_dir.join(".mt");
    std::fs::create_dir_all(&mt)?;
    let _ = set_hidden(&mt);

    // 曲库级设置：目录布局 + 名称 + 结构方案。
    let mut lib = default_library_settings(library_dir);
    if LibraryConfig::is_valid_scheme(scheme) {
        lib.library.file_scheme = scheme.to_string();
    }
    // 曲库名称：用户指定则用之，否则回退到曲库目录名，保证每个曲库都有可读名称。
    let trimmed = name.trim();
    lib.library.name = if trimmed.is_empty() {
        library_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("我的曲库")
            .to_string()
    } else {
        // 限制长度，避免极端输入影响 UI / 配置可读性。
        trimmed.chars().take(120).collect()
    };
    // NCM 开关 = tool_ready()：.mt/tools/ 下检测到工具则用其路径；否则置空（未配置，.ncm 暂存 pendingNcm）。
    lib.paths.tools_dir = mt.join("tools");
    let detected = detect_ncm_tool(&mt).unwrap_or_else(PathBuf::new);

    // 合并：app 级沿用当前 app 配置（保留 NCM 工具 / 导入源 / 界面偏好等），曲库级用新建的。
    // 用兼容感知加载：坏配置不再静默清空，而是先备份原文件再重建。
    let app_path = app_config_path();
    let (mut app, app_compat, _) = load_app_settings_with_compat(&app_path);
    if app_compat.is_conflict() {
        let bak = backup_conflict(&app_path);
        eprintln!(
            "[config] app 配置存在逻辑冲突，已备份并重建。原因：{}{}",
            app_compat.reason(),
            bak.map(|b| format!("（备份：{}）", b.display())).unwrap_or_default()
        );
    }
    if !detected.as_os_str().is_empty() && !app.ncm.tool_ready() {
        app.ncm.tool_path = detected;
    }
    app.last_library_dir = Some(library_dir.to_path_buf());

    let cfg = Config::merge(&app, &lib);
    cfg.ensure_dirs()?; // 建 app_data/download/download_ing/ncm/pendingNcm/tools
    cfg.save_library(&library_config_path(library_dir))?;
    cfg.save_app(&app_path)?;
    std::fs::write(mt.join("README.txt"), README_TEXT)?;
    // 建库（建表）。
    db::open(&cfg)?;
    Ok(cfg)
}

/// 构建指定曲库对应的曲库级配置（绝对路径，不依赖 CWD，docs/05 §3/§5）。
/// `name` 为空时回退到曲库目录名，保证每个曲库都有可读名称。
pub fn build_library_config(library_dir: &Path, scheme: &str, name: &str) -> Config {
    let mut cfg = default_config();
    let mt = library_dir.join(".mt");
    cfg.paths.app_data = mt.clone();
    cfg.paths.download_root = library_dir.to_path_buf();
    cfg.paths.download_dir = library_dir.join("download");
    cfg.paths.download_ing_dir = library_dir.join("download").join("ing");
    cfg.paths.ncm_raw_dir = library_dir.join("download").join("ncm");
    cfg.paths.tools_dir = mt.join("tools");
    if LibraryConfig::is_valid_scheme(scheme) {
        cfg.library.file_scheme = scheme.to_string();
    }
    let trimmed = name.trim();
    cfg.library.name = if trimmed.is_empty() {
        library_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("我的曲库")
            .to_string()
    } else {
        trimmed.chars().take(120).collect()
    };
    cfg.ncm.tool_path = detect_ncm_tool(&mt).unwrap_or_else(PathBuf::new);
    cfg
}

/// 曲库根 = .mt 父目录，运行时推导（不持久化绝对路径，移动硬盘换盘符只需重新指定）。
pub fn resolve_download_root(app_data: &Path) -> PathBuf {
    app_data.parent().unwrap_or(app_data).to_path_buf()
}

/// 在 .mt/tools/ 下检测 ncm 解密工具（候选名）。找到返回其路径。
pub fn detect_ncm_tool(app_data: &Path) -> Option<PathBuf> {
    let tools = app_data.join("tools");
    const CANDIDATES: &[&str] = &["ncmdump.exe", "ncmdump", "ncmpy.exe"];
    for name in CANDIDATES {
        let p = tools.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// 把 .mt 设为隐藏目录（Windows 用 attrib；其他平台忽略）。best-effort。
#[cfg(windows)]
fn set_hidden(path: &Path) -> std::io::Result<()> {
    // 用 silent_command 避免 GUI 进程调用 attrib 时弹出控制台窗口。
    let _ = crate::silent_command("attrib")
        .arg("+h")
        .arg(path)
        .output();
    Ok(())
}
#[cfg(not(windows))]
fn set_hidden(_path: &Path) {}

const README_TEXT: &str = r#"K曲库 曲库数据目录
=======================

本目录（.mt）由 K曲库 自动创建并维护，是该曲库的隐藏数据目录。

存放内容：
- config.toml   本曲库的配置文件（曲库名称、文件结构方案、目录布局）
- library.db    本曲库的音频索引数据库（扫描抽取的音质元数据）
- app.log       运行日志
- tools/        NCM 解密工具（ncmdump.exe）存放处，应用会自动检测并使用

说明：
- 本目录与音乐文件相互独立。删除本目录【不会】删除你的音乐文件。
- 当你不再需要该曲库、或要移除/迁移曲库时，可以【安全删除整个 .mt 文件夹】，
  这只会清除该曲库的索引与配置，音乐文件不受影响。
- 应用自身的设置（主题、NCM 工具、导入源等）不在本目录，而在程序所在目录的 kqk-app.toml。
- 如需使用 NCM 解密功能，请把 ncmdump.exe 放入 tools/ 目录
  （或在应用内「指定 NCM 工具」，应用会自动复制到此目录）。
"#;

impl PathsConfig {
    /// 无 ncm 工具时待转换 .ncm 的暂存目录（= download_dir/pendingNcm）。
    pub fn pending_ncm_dir(&self) -> PathBuf {
        self.download_dir.join("pendingNcm")
    }
}

/// 清洗文件名组件：去除路径分隔符、控制字符，压缩空白，避免越界或空名。
fn sanitize_component(s: &str) -> String {
    let out: String = s
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        "Unknown".to_string()
    } else {
        // 限制单段长度，避免 Windows MAX_PATH 问题
        trimmed.chars().take(80).collect()
    }
}

/// 按 file_scheme 计算文件在曲库根下的相对目录（不含扩展名）。
/// 方案：
/// - artist_album: <artist>/<album>/<title>
/// - album:        <album>/<title>
/// - artist:       <artist>/<title>
/// - flat:         <title> - <artist>
/// 缺失元数据以 Unknown 兜底；调用方负责拼接扩展名与最终落点。
pub fn target_path(scheme: &str, artist: &str, album: &str, title: &str) -> PathBuf {
    let a = sanitize_component(artist);
    let al = sanitize_component(album);
    let t = sanitize_component(title);
    match scheme {
        "album" => PathBuf::from(&al).join(&t),
        "artist" => PathBuf::from(&a).join(&t),
        "flat" => PathBuf::from(format!("{} - {}", t, a)),
        _ => PathBuf::from(&a).join(&al).join(&t), // artist_album（默认）
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn build_library_config_name_falls_back_to_dir_stem() {
        // 名称为空时回退到曲库目录名，保证每个曲库都有可读名称。
        let dir = Path::new("/music/My Cool Library");
        let cfg = build_library_config(dir, "artist_album", "");
        assert_eq!(cfg.library.name, "My Cool Library");
    }

    #[test]
    fn build_library_config_name_uses_explicit_and_scheme() {
        let dir = Path::new("/music/foo");
        let cfg = build_library_config(dir, "flat", "我的古典曲库");
        assert_eq!(cfg.library.name, "我的古典曲库");
        assert_eq!(cfg.library.file_scheme, "flat");
    }

    #[test]
    fn library_name_is_backward_compatible() {
        // 旧配置（无 library.name 字段）用 serde(default) 解析不应失败，且回退为空串。
        let toml = r#"
            [paths]
            app_data = "./app/data"
            download_root = "./musiclibrary"
            download_dir = "./musiclibrary/download"
            download_ing_dir = "./musiclibrary/download/ing"
            ncm_raw_dir = "./musiclibrary/download/ncm"
            tools_dir = "./tools"
            [netease]
            library_dir = ""
            [ncm]
            tool_path = ""
            command_template = "{tool} \"{input}\""
            [scanner]
            audio_meta = "lofty"
            match_strategy = "clean_name_db"
            [import]
            default_mode = "copy"
            [library]
            file_scheme = "artist_album"
            [server]
            host = "127.0.0.1"
            port = 0
            [logging]
            level = "info"
            path = "./app/data/app.log"
        "#;
        let cfg: LibrarySettings = toml::from_str(toml).expect("旧配置应能正常解析");
        assert_eq!(cfg.library.name, "");
        assert_eq!(cfg.library.file_scheme, "artist_album");
    }

    #[test]
    fn split_and_merge_roundtrip_preserves_sections() {
        // 合并 → 拆分 → 合并应完全等价，且 app/曲库两段各自独立。
        let mut cfg = default_config();
        cfg.library.name = "曲库A".into();
        cfg.ncm.tool_path = PathBuf::from("C:/tools/ncmdump.exe");
        cfg.netease.library_dir = PathBuf::from("D:/Music");
        cfg.last_library_dir = Some(PathBuf::from("E:/Lib"));

        let (app, lib) = cfg.split();
        assert_eq!(app.ncm.tool_path, PathBuf::from("C:/tools/ncmdump.exe"));
        assert_eq!(app.netease.library_dir, PathBuf::from("D:/Music"));
        assert_eq!(app.last_library_dir, Some(PathBuf::from("E:/Lib")));
        assert_eq!(lib.library.name, "曲库A");
        // 曲库级不应携带 app 级字段。
        assert!(!lib.paths.tools_dir.as_os_str().is_empty());

        let merged = Config::merge(&app, &lib);
        assert_eq!(merged.library.name, "曲库A");
        assert_eq!(merged.ncm.tool_path, PathBuf::from("C:/tools/ncmdump.exe"));
        assert_eq!(merged.last_library_dir, Some(PathBuf::from("E:/Lib")));
    }

    #[test]
    fn app_settings_file_keeps_library_path_and_excludes_library_sections() {
        // app 配置文件含当前曲库路径，且不含曲库级段落（paths/library）。
        let mut cfg = default_config();
        cfg.last_library_dir = Some(PathBuf::from("E:/Lib"));
        let (app, _) = cfg.split();
        let text = toml::to_string_pretty(&app).unwrap();
        assert!(text.contains("last_library_dir"));
        assert!(!text.contains("[paths]"));
        assert!(!text.contains("[library]"));

        // 曲库级配置文件只含 paths/library。
        let (_, lib) = cfg.split();
        let text = toml::to_string_pretty(&lib).unwrap();
        assert!(text.contains("[paths]"));
        assert!(text.contains("[library]"));
        assert!(!text.contains("last_library_dir"));
    }

    #[test]
    fn legacy_full_config_yields_app_sections() {
        // 旧版曲库配置里混放的 app 级段落可被迁移逻辑读出。
        let toml = r#"
            [paths]
            app_data = "./app/data"
            download_root = "./musiclibrary"
            download_dir = "./musiclibrary/download"
            download_ing_dir = "./musiclibrary/download/ing"
            ncm_raw_dir = "./musiclibrary/download/ncm"
            tools_dir = "./tools"
            [netease]
            library_dir = "D:/Music"
            [ncm]
            tool_path = "C:/tools/ncmdump.exe"
            command_template = "{tool} \"{input}\""
            [ui]
            track_columns = ["name", "artist"]
        "#;
        let legacy: LegacyFullConfig = toml::from_str(toml).expect("旧配置应能解析出 app 级段落");
        assert_eq!(legacy.netease.unwrap().library_dir, PathBuf::from("D:/Music"));
        assert_eq!(legacy.ncm.unwrap().tool_path, PathBuf::from("C:/tools/ncmdump.exe"));
        assert_eq!(legacy.ui.unwrap().track_columns, vec!["name", "artist"]);
    }

    // ========================================================
    // 版本兼容判定与「保留式」配置维护（2026-11-04）
    // ========================================================

    fn meta(cv: &str, min: &str) -> ConfigMeta {
        ConfigMeta {
            config_version: cv.to_string(),
            min_compatible_version: min.to_string(),
        }
    }

    #[test]
    fn compat_same_version_is_compatible() {
        let m = meta(crate::app::APP_VERSION, crate::app::MIN_COMPATIBLE_VERSION);
        assert_eq!(assess_compat(&m, None), ConfigCompat::Compatible);
    }

    #[test]
    fn compat_older_config_is_compatible_and_content_preserved() {
        // 旧版本生成的配置 → 兼容（内容沿用，仅更新版本信息）。
        let m = meta("0.1.0", "0.1.0");
        assert_eq!(assess_compat(&m, None), ConfigCompat::Compatible);
    }

    #[test]
    fn compat_missing_meta_is_legacy() {
        // 无 [meta] 段 → 旧格式：内容沿用并补写版本信息。
        assert_eq!(assess_compat(&ConfigMeta::default(), None), ConfigCompat::Legacy);
    }

    #[test]
    fn compat_newer_config_is_conflict_with_reason() {
        let m = meta("99.0.0", "0.2.0");
        let c = assess_compat(&m, None);
        assert!(c.is_conflict());
        assert!(c.reason().contains("99.0.0"), "冲突原因应含配置版本号：{}", c.reason());
    }

    #[test]
    fn compat_min_version_above_current_is_conflict_with_reason() {
        let m = meta(crate::app::APP_VERSION, "99.0.0");
        let c = assess_compat(&m, None);
        assert!(c.is_conflict());
        assert!(c.reason().contains("99.0.0"), "冲突原因应含最低兼容版本：{}", c.reason());
    }

    #[test]
    fn compat_parse_error_is_conflict_with_reason() {
        let c = assess_compat(&ConfigMeta::default(), Some("expected `=`"));
        assert!(c.is_conflict());
        assert!(c.reason().contains("解析失败"), "冲突原因应说明解析失败：{}", c.reason());
    }

    #[test]
    fn inject_meta_preserves_comments_and_unknown_keys() {
        let orig = r#"# 用户自备配置，请勿删除
last_library_dir = 'D:\music'

[netease]
library_dir = "D:/NetEase/music"

# 手工加的未知键，必须原样保留
my_custom = "keep me"
"#;
        let m = meta("0.2.0", "0.2.0");
        let out = inject_meta(orig, &m);
        // 注释、未知键、既有内容全部保留
        assert!(out.contains("# 用户自备配置，请勿删除"), "注释应保留");
        assert!(out.contains("# 手工加的未知键，必须原样保留"), "注释应保留");
        assert!(out.contains("my_custom = \"keep me\""), "未知键应保留");
        assert!(out.contains("last_library_dir = 'D:\\music'"), "既有内容应保留");
        assert!(out.contains("library_dir = \"D:/NetEase/music\""), "既有内容应保留");
        // 版本信息已补写
        assert!(out.contains("[meta]"), "应追加 [meta] 段");
        assert!(out.contains("config_version = \"0.2.0\""), "应写入生成版本");
        assert!(out.contains("min_compatible_version = \"0.2.0\""), "应写入最低兼容版本");
        // 结果仍是合法 TOML，且解析得到同样的业务字段
        let parsed: AppSettings = toml::from_str(&out).expect("注入后应仍是合法 TOML");
        assert_eq!(parsed.netease.library_dir, PathBuf::from("D:/NetEase/music"));
        assert_eq!(parsed.meta.config_version, "0.2.0");
    }

    #[test]
    fn inject_meta_updates_existing_meta_section_in_place() {
        let orig = r#"[meta]
config_version = "0.1.0"
min_compatible_version = "0.1.0"

[netease]
library_dir = ""
"#;
        let out = inject_meta(orig, &meta("0.2.0", "0.2.0"));
        assert!(out.contains("config_version = \"0.2.0\""));
        assert!(out.contains("min_compatible_version = \"0.2.0\""));
        assert!(!out.contains("0.1.0"), "旧版本号应被替换");
        assert_eq!(out.matches("[meta]").count(), 1, "不应重复追加 [meta] 段");
    }

    #[test]
    fn upsert_top_level_keeps_rest_and_inserts_before_first_table() {
        let orig = r#"[netease]
library_dir = "D:/x"
"#;
        let out = upsert_top_level(orig, "last_library_dir", "\"D:/lib\"");
        assert!(out.starts_with("last_library_dir = \"D:/lib\""), "应插入到首个表头之前");
        assert!(out.contains("library_dir = \"D:/x\""), "其余内容应保留");
        let parsed: AppSettings = toml::from_str(&out).expect("应为合法 TOML");
        assert_eq!(parsed.last_library_dir, Some(PathBuf::from("D:/lib")));
    }

    #[test]
    fn partial_config_is_tolerated_and_keeps_present_values() {
        // 缺字段的残缺配置不应整份作废：能解析出的字段必须保留。
        let txt = r#"
[netease]
library_dir = "D:/NetEase/music"
"#;
        let parsed: AppSettings = toml::from_str(txt).expect("残缺配置应能容错解析");
        assert_eq!(parsed.netease.library_dir, PathBuf::from("D:/NetEase/music"));
        // 缺失的段回退默认
        assert_eq!(parsed.ncm.command_template, default_command_template());
        assert_eq!(parsed.import.default_mode, "copy");
        assert_eq!(parsed.server.host, "127.0.0.1");
        assert_eq!(parsed.logging.level, "info");
        assert_eq!(parsed.ui.track_columns, default_track_columns());
    }

    #[test]
    fn app_settings_roundtrip_keeps_meta_section() {        let mut app = default_app_settings();
        app.netease.library_dir = PathBuf::from("D:/NetEase/music");
        app.meta = ConfigMeta::current();
        let text = toml::to_string_pretty(&app).expect("应能序列化");
        assert!(text.contains("[meta]"), "序列化结果应含 [meta] 段");
        let back: AppSettings = toml::from_str(&text).expect("应能反序列化");
        assert_eq!(back.meta.config_version, crate::app::APP_VERSION);
        assert_eq!(back.meta.min_compatible_version, crate::app::MIN_COMPATIBLE_VERSION);
        assert_eq!(back.netease.library_dir, PathBuf::from("D:/NetEase/music"));
    }

    #[test]
    fn salvage_keeps_stable_path_fields_on_conflict() {
        // 冲突重建时仍应抢救路径类稳定字段，避免用户重新指定曲库 / NCM 工具 / 导入源。
        let txt = r#"
last_library_dir = 'D:\music\lib'

[meta]
config_version = "99.0.0"

[netease]
library_dir = "D:/NetEase/music"

[ncm]
tool_path = "C:/tools/ncmdump.exe"
command_template = "{tool} \"{input}\""

[ui]
track_columns = ["name", "artist"]
"#;
        let s = salvage_app_settings(txt);
        assert_eq!(s.last_library_dir, Some(PathBuf::from("D:\\music\\lib")));
        assert_eq!(s.netease.library_dir, PathBuf::from("D:/NetEase/music"));
        assert_eq!(s.ncm.tool_path, PathBuf::from("C:/tools/ncmdump.exe"));
        // 语义可能变化的复杂结构不抢救（回到默认）。
        assert_eq!(s.ui.track_columns, default_track_columns());
    }

    // ---------- 界面偏好（布局风格 + 列宽比例）----------

    #[test]
    fn ui_defaults_follow_spec() {
        // 需求：主界面默认横向，播放列表/专辑/歌手默认纵向。
        let ui = UiConfig::default();
        assert_eq!(ui.layout_for("library"), LAYOUT_STYLE_HORIZONTAL);
        assert_eq!(ui.layout_for("playlists"), LAYOUT_STYLE_VERTICAL);
        assert_eq!(ui.layout_for("albums"), LAYOUT_STYLE_VERTICAL);
        assert_eq!(ui.layout_for("artists"), LAYOUT_STYLE_VERTICAL);
        // 未知视图回退横向（不 panic、不给越界值）。
        assert_eq!(ui.layout_for("unknown"), LAYOUT_STYLE_HORIZONTAL);
        // 八列占比之和恒为 100。
        let sum: f64 = all_track_column_keys().iter().map(|k| ui.width_for(k)).sum();
        assert!((sum - 100.0).abs() < 0.01, "默认列宽占比之和应为 100，实为 {sum}");
    }

    #[test]
    fn created_at_column_is_hidden_by_default() {
        let def = default_track_columns();
        assert!(all_track_column_keys().contains(&"created_at".to_string()),
                "创建时间必须仍是可选列（设置面板里能勾到）");
        assert!(def.contains(&"imported_at".to_string()), "导入时间默认显示");
        assert!(!def.contains(&"created_at".to_string()), "创建时间默认不显示");
        assert_eq!(def.len(), all_track_column_keys().len() - 1);
    }

    #[test]
    fn ui_sanitize_migrates_legacy_track_columns() {
        // 旧配置（未显式保存过）：保留用户自定义的列顺序 / 子集，补上「导入时间」，
        // 但绝不补「创建时间」。且迁移必须幂等。
        let mut ui = UiConfig::default();
        ui.track_columns = vec!["name".into(), "album".into()];
        ui.track_columns_saved = false;
        ui.sanitize();
        assert_eq!(ui.track_columns,
                   vec!["name".to_string(), "album".to_string(), "imported_at".to_string()]);
        ui.sanitize();
        assert_eq!(ui.track_columns,
                   vec!["name".to_string(), "album".to_string(), "imported_at".to_string()]);
    }

    #[test]
    fn ui_sanitize_strips_created_at_from_unsaved_config() {
        // 早期版本曾把 8 列全部写进配置；未显式保存过 → 回退到「创建时间隐藏」的新默认。
        let mut ui = UiConfig::default();
        ui.track_columns = all_track_column_keys();
        ui.track_columns_saved = false;
        ui.sanitize();
        assert!(!ui.track_columns.contains(&"created_at".to_string()));
        assert!(ui.track_columns.contains(&"imported_at".to_string()));
    }

    #[test]
    fn ui_sanitize_respects_explicitly_saved_columns() {
        // 用户通过「应用显示设置」显式保存过 → 打开创建时间 / 关掉导入时间都要被尊重。
        let mut ui = UiConfig::default();
        ui.track_columns = vec!["name".into(), "created_at".into()];
        ui.track_columns_saved = true;
        ui.sanitize();
        assert_eq!(ui.track_columns, vec!["name".to_string(), "created_at".to_string()]);
        // 显式保存过也要过滤非法 key 与重复 key。
        ui.track_columns = vec!["name".into(), "bogus".into(), "name".into(), "album".into()];
        ui.sanitize();
        assert_eq!(ui.track_columns, vec!["name".to_string(), "album".to_string()]);
    }

    #[test]
    fn ui_set_layout_rejects_invalid_values() {
        let mut ui = UiConfig::default();
        assert!(ui.set_layout("library", LAYOUT_STYLE_VERTICAL));
        assert_eq!(ui.layout_for("library"), LAYOUT_STYLE_VERTICAL);
        // 非法风格值（0 / 3）必须被忽略。
        assert!(!ui.set_layout("library", 0));
        assert!(!ui.set_layout("library", 3));
        assert_eq!(ui.layout_for("library"), LAYOUT_STYLE_VERTICAL);
        // 非法 view 必须被忽略。
        assert!(!ui.set_layout("nope", LAYOUT_STYLE_HORIZONTAL));
    }

    #[test]
    fn ui_sanitize_normalizes_out_of_range_values() {
        let mut ui = UiConfig::default();
        ui.layout_library = 9; // 非法风格
        ui.width_name = 1.0; // 低于下限 4%
        ui.width_artist = 60.0;
        ui.width_album = 60.0;
        ui.sanitize();
        assert_eq!(ui.layout_for("library"), LAYOUT_STYLE_HORIZONTAL);
        // 归一化后每列 ≥ MIN_COLUMN_WIDTH_PCT，且总和为 100。
        let sum: f64 = all_track_column_keys().iter().map(|k| ui.width_for(k)).sum();
        assert!((sum - 100.0).abs() < 0.01, "归一化后总和应为 100，实为 {sum}");
        for k in all_track_column_keys() {
            assert!(
                ui.width_for(&k) >= MIN_COLUMN_WIDTH_PCT - 0.001,
                "列 {k} 归一化后仍低于下限：{}",
                ui.width_for(&k)
            );
        }
    }

    #[test]
    fn ui_legacy_config_parses_with_defaults() {
        // 旧配置文件只有 [ui] track_columns → 新字段缺失时应回退默认值而非解析失败。
        let txt = r#"
[ui]
track_columns = ["name", "album"]
"#;
        let doc: toml::Table = toml::from_str(txt).expect("旧配置应能解析");
        let ui: UiConfig = doc["ui"].clone().try_into().expect("[ui] 表应能转成 UiConfig");
        assert_eq!(ui.track_columns, vec!["name".to_string(), "album".to_string()]);
        assert_eq!(ui.layout_for("library"), LAYOUT_STYLE_HORIZONTAL);
        assert_eq!(ui.layout_for("artists"), LAYOUT_STYLE_VERTICAL);
        let sum: f64 = all_track_column_keys().iter().map(|k| ui.width_for(k)).sum();
        assert!((sum - 100.0).abs() < 0.01);
    }

    #[test]
    fn upsert_ui_section_preserves_comments_and_unknown_keys() {
        let orig = r#"# 这是用户手写的注释
last_library_dir = 'D:\music\lib'

[meta]
config_version = "0.3.0"
min_compatible_version = "0.3.0"

[netease]
library_dir = "D:/NetEase/music"

[ui]
track_columns = ["name", "artist"]
# ui 段内的注释

[custom]
my_own_key = 42
"#;
        let mut ui = UiConfig::default();
        ui.set_layout("library", LAYOUT_STYLE_VERTICAL);
        ui.set_width("name", 40.0);
        let out = upsert_ui_section(orig, &ui);
        // 注释与未知键原样保留。
        assert!(out.contains("# 这是用户手写的注释"));
        assert!(out.contains("# ui 段内的注释"));
        assert!(out.contains("my_own_key = 42"));
        assert!(out.contains("last_library_dir = 'D:\\music\\lib'"));
        assert!(out.contains("[netease]"));
        // [ui] 段被替换为新值。
        assert!(out.contains("layout_library = 2"));
        assert!(out.contains("width_name = 40"));
        // 结果应能被重新解析回来。
        let doc: toml::Table = toml::from_str(&out).expect("替换后的配置应能解析");
        let back: UiConfig = doc["ui"].clone().try_into().expect("[ui] 表应能转成 UiConfig");
        assert_eq!(back.layout_for("library"), LAYOUT_STYLE_VERTICAL);
    }

    #[test]
    fn upsert_ui_section_keeps_multiline_array_intact() {
        // 旧配置里 track_columns 写成多行数组 → 替换时不能把续行留在原地变成语法错误。
        let orig = "[ui]\ntrack_columns = [\n    \"name\",\n    \"artist\",\n]\nwidth_name = 30.0\n";
        let mut ui = UiConfig::default();
        ui.set_width("name", 42.0);
        let out = upsert_ui_section(orig, &ui);
        let doc: toml::Table = toml::from_str(&out).expect("多行数组场景仍应能解析");
        let back: UiConfig = doc["ui"].clone().try_into().expect("[ui] 表应能转成 UiConfig");
        assert_eq!(back.width_for("name"), 42.0);
        assert_eq!(
            back.track_columns.len(),
            default_track_columns().len(),
            "应写成完整的默认可见列（创建时间默认隐藏）"
        );
    }

    #[test]
    fn upsert_ui_section_appends_when_missing() {
        let orig = "last_library_dir = 'D:\\music\\lib'\n";
        let ui = UiConfig::default();
        let out = upsert_ui_section(orig, &ui);
        assert!(out.contains("[ui]"));
        assert!(out.contains("last_library_dir = 'D:\\music\\lib'"));
        let back: UiConfig = toml::from_str(&out).expect("应能解析");
        assert_eq!(back.layout_for("playlists"), LAYOUT_STYLE_VERTICAL);
    }

    #[test]
    fn save_ui_in_place_writes_and_reroundtrips() {
        let dir = std::env::temp_dir().join("kqk_ui_test_save");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("应能建临时目录");
        let path = dir.join("kqk-app.toml");
        std::fs::write(
            &path,
            "# 保留我\nlast_library_dir = 'D:\\music\\lib'\n\n[ui]\ntrack_columns = [\"name\"]\n",
        )
        .expect("应能写初始文件");

        let mut ui = UiConfig::default();
        ui.set_layout("artists", LAYOUT_STYLE_HORIZONTAL);
        ui.set_width("album", 30.0);
        save_ui_in_place(&path, &ui).expect("保存应成功");

        let text = std::fs::read_to_string(&path).expect("应能读回");
        assert!(text.contains("# 保留我"), "注释应保留");
        assert!(text.contains("[meta]"), "应写入 [meta] 段");
        let doc: toml::Table = toml::from_str(&text).expect("落盘文件应能解析");
        let back: UiConfig = doc["ui"].clone().try_into().expect("[ui] 表应能转成 UiConfig");
        assert_eq!(back.layout_for("artists"), LAYOUT_STYLE_HORIZONTAL);
        assert_eq!(back.layout_for("library"), LAYOUT_STYLE_HORIZONTAL);
        assert_eq!(back.layout_for("albums"), LAYOUT_STYLE_VERTICAL);
        let sum: f64 = all_track_column_keys().iter().map(|k| back.width_for(k)).sum();
        assert!((sum - 100.0).abs() < 0.01, "落盘后占比之和应为 100，实为 {sum}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
