// K music 前端（原生 JS + Material 风样式）。所有接口走相对路径（WebView 与后端同源）。
// 设计要点：网易云服务端登录/下载已移除（2026-10-29）；本地导入源仅「从本地网易云曲库更新」按钮。

const $ = (id) => document.getElementById(id);
const audio = $('audio');

// 交互增强 b 级：托盘菜单 / 媒体键的顶层命令入口（必须定义在 boot 之外，供 Rust 侧 window.eval 调用）。
// 白名单：toggle-play / prev / next / toggle-mute / show-settings；其余命令被忽略。
window.kqkCommand = function (cmd) {
  switch (cmd) {
    case 'toggle-play':
      if (!audio.src) { if (state.playback.queue.length) playCurrent(); }
      else if (audio.paused) { audio.play().catch(() => {}); $('btn-play').textContent = '⏸'; }
      else { audio.pause(); $('btn-play').textContent = '▶'; }
      break;
    case 'prev': stepQueue(-1, false); break;
    case 'next': stepQueue(1, false); break;
    case 'toggle-mute':
      state.muted = !state.muted;
      if (!state.muted && state.volume === 0) state.volume = 100;
      applyVolume();
      savePanePrefs({ muted: state.muted, volume: Math.round(state.volume) });
      break;
    case 'show-settings': {
      openSettings();
      activateSettingsTab('appearance');
      const sc = document.querySelector('.settings-content');
      const sec = $('ap-tray-section');
      if (sc && sec) {
        const d = sec.getBoundingClientRect().top - sc.getBoundingClientRect().top;
        sc.scrollTop = Math.max(0, sc.scrollTop + d - 8);
        sec.classList.remove('focus-flash'); void sec.offsetWidth; sec.classList.add('focus-flash');
      }
      break;
    }
    default: break;
  }
};

// 歌曲列表全部可选列（顺序即「设置 → 列表」里的默认顺序）。
const ALL_COLUMN_KEYS = [
  'name', 'artist', 'album', 'quality_tier', 'duration_ms', 'file_size',
  'imported_at', 'created_at',
];
// 默认可见列：导入时间显示、创建时间隐藏（创建时间由设置面板勾选开启）。
// 定义必须早于 state，避免 const 暂时性死区。
const DEFAULT_VISIBLE_COLUMN_KEYS = ALL_COLUMN_KEYS.filter((k) => k !== 'created_at');

// 自动（无名）播放列表上限 n：app 级设置，默认 20 / 下限 1 / 上限 200。
// 只允许 autoPlaylistLimit() 这个唯一入口读取，且只用于「生成队列」，绝不用于表格 / 导出。
const AUTO_COUNT_DEFAULT = 20;
const AUTO_COUNT_MIN = 1;
const AUTO_COUNT_MAX = 200;
function clampAutoCount(v) {
  const n = Number(v);
  if (!isFinite(n) || n < AUTO_COUNT_MIN) return AUTO_COUNT_DEFAULT;
  return Math.min(Math.round(n), AUTO_COUNT_MAX);
}
function autoPlaylistLimit() { return clampAutoCount(state.autoPlaylistCount); }

const state = {
  library_configured: false,
  library_dir: '',
  library_name: '',
  // 应用元信息（来自 /api/about）：名称、版本号、作者、联系方式、许可证等。
  about: null,
  netease_library_dir: '',
  // 配置文件版本状态：生成版本 / 最低兼容版本 / 兼容判定 / 冲突原因。
  config_version: '',
  config_min_version: '',
  config_compat: '',
  config_conflict: '',
  library_config_version: '',
  library_config_compat: '',
  library_config_conflict: '',
  app_version: '',
  // FR-30：联网四键（权威源 = /api/status 下发的 app 级 [ui]，默认全部关闭）。
  networkEnabled: false,
  updateCheckEnabled: false,
  updateSource: 'github',
  updateInterval: 'daily',
  // 「稍后」只在本次会话隐藏提示条；重启后按缓存结果重现（A6）。
  updateBarDismissed: false,
  file_scheme: 'artist_album',
  ncm_ready: false,
  ncm_enabled: true,
  ncm_tool: '',
  // v0.14.0：添加曲源探测到网易云时的 NCM 引导，会话内只弹一次。
  ncmGuideShown: false,
  // 插件页状态（GET /api/plugins）。
  plugins: [],
  // FR-22：未决重复组数（W15 只统计当前曲库、按组数）与挂起目录 / 独立日志路径。
  pendingDuplicates: 0,
  duplicatesDir: '',
  duplicatesLogPath: '',
  duplicateGroups: [],
  tracks: [],
  // 全部曲库视图的筛选状态（会话内有效，不持久化；音质 quality / 仅看收藏 favorite）。
  tracksQuery: '',
  // v0.16.2：曲库列表加载状态（idle/loading/ready/empty/error）与错误信息。
  // 失败时保留 state.tracks 旧数据，不清空表格；状态条显示在曲库表格上方。
  trackListState: 'idle',
  trackListError: '',
  // #33：曲库筛选只保留音质；「已收藏」入口已删除（收藏是第 5 个一级视图）。
  // v0.17.7：unavailable=true = 仅看不可访问（正向筛选；默认 false = 全部含 ⚠ 标记项）。
  trackFilters: { quality: '', unavailable: false },
  // #31/#32：「喜爱」= 第 5 个一级视图（activeTab==='favorites'），与专辑/歌手同级。
  // 集合是实时推导的播放来源快照，绝不写入 playlists 表；无二级栏，直接复用曲库表格。
  favoriteTracks: [],
  favoriteCount: 0,
  current: null,
  // 分视图选中态：五个页面各自的「曲级选中」互不干扰。
  // value = { track: 列表中的曲目摘要, trx: /api/track 返回的完整详情 } 或 null。
  // 旧实现只有一个全局 state.current，切页后详情栏会残留上一页的数据。
  // v0.12.7：第 6 个视图「曲源」加入分视图选中态，避免跨页串详情。
  viewSelection: { library: null, playlists: null, albums: null, artists: null, favorites: null, sources: null },
  // 每页当前是否有可展示的详情内容（渲染时更新，供可见性仲裁使用）。
  // 无内容 → 该页的详情区（右侧栏 / 内联条）整体隐藏，不渲染占位内容。
  detailShown: { library: false, playlists: false, albums: false, artists: false, favorites: false, sources: false },
  activeTab: 'library',
  // 播放列表（本地曲库内用户自建，local_id 体系）。
  playlists: [],
  currentPlaylist: null,
  playlistTracks: [],
  // 专辑 / 歌手（按元数据维度的分组浏览，与播放列表同构：列表 → 选中后歌曲）。
  albums: [],
  currentAlbum: null,
  albumTracks: [],
  // #27：专辑集合的呈现形态（'list' 文字行 / 'grid' 封面卡片）。会话内有效、不落盘；
  // 只影响二级栏 al-list 的渲染，al-content 曲目表格 / 列宽 / 播放队列完全不变。
  albumView: 'list',
  artists: [],
  currentArtist: null,
  artistTracks: [],
  // v0.12.7 曲源页：二级分组（按 source_kind 归属）、当前选中曲源、三级曲目。
  // 以 source.id 为 key，改名只影响 name，不影响已导入歌曲归属。
  sourceGroups: [],
  currentSource: null,
  sourceTracks: [],
  // 曲源总览：每行状态（idle / running / done / error）与串行更新汇总，仅会话内有效。
  sourceStatus: {},
  dlTimer: null,
  // 散落 .ncm 处理
  ncmOrganizePaths: [],
  ncmDismissed: new Set(), // 用户点过「暂不处理」的路径（本次会话内不再自动弹）
  // 歌曲列表显示列：columns = 全列 key 的有序列表（含隐藏）；visible = 可见列 key 集合。
  // 默认「导入时间」显示、「创建时间」隐藏；后者由「设置 → 列表 → 歌曲列表显示项」勾选开启。
  columns: ['name', 'artist', 'album', 'quality_tier', 'duration_ms', 'file_size', 'imported_at', 'created_at'],
  visible: new Set(DEFAULT_VISIBLE_COLUMN_KEYS),
  // 布局风格：按页面独立设置。1 = 横向（详情在右侧独立栏）；2 = 纵向（详情内联到列表上方）。
  // 默认：主界面横向，播放列表 / 专辑 / 歌手纵向。
  layoutStyles: { library: 1, playlists: 2, albums: 2, artists: 2 },
  // 歌曲列表各列宽度占比（%），八列之和恒为 100（持久化在配置文件 [ui] 段）。
  columnWidths: { name: 24, artist: 16, album: 18, quality_tier: 6, duration_ms: 6, file_size: 8, imported_at: 11, created_at: 11 },
  // 自动播放列表上限（由 /api/status 读回；设置 → 播放 可改）。
  autoPlaylistCount: 20,
  // 歌曲筛选设置（app 级 [import]；0 = 不启用）。
  importPrefs: { minSizeKb: 500, minDurationSec: 30, autoUpdateSourcesOnStart: false },
  // FR-24：侧栏宽度（px）/ 收缩态 / 播放器音量，均为 app 级 [ui] 偏好（换曲库不变）。
  sidebarWidth: 144,
  groupWidth: 230,
  sidebarCollapsed: false,
  groupCollapsed: false,
  volume: 100,
  muted: false,
  // FR-25：歌曲信息栏（含封面）收起态，app 级 [ui] 偏好。
  detailCollapsed: false,
  // 歌词：{ timed, lines: [{t, text}], plain }；lyricsIndex = 当前高亮行序号。
  lyrics: null,
  lyricsIndex: -1,
  lyricsSource: '',
  // #29：全屏「正在播放 / 歌词」覆盖层开关（会话内，不落 [ui] / playstate）。
  nowViewOpen: false,
  // 当前歌词所属曲目，避免打开全屏层时重复请求 /api/lyrics。
  lyricsLocalId: null,
};

// 曲名单元格内的「正在播放」音符（SVG 结构，切歌增量同步共用）。
const PLAY_MARK_SVG = '<svg viewBox="0 0 12 12" aria-hidden="true"><ellipse cx="4.1" cy="8.9" rx="2.7" ry="2.2" transform="rotate(-22 4.1 8.9)"/><rect x="6.1" y="1.4" width="1.35" height="7.7" rx=".6"/></svg>';
const PLAY_MARK_HTML = `<span class="play-mark" title="正在播放" aria-label="正在播放">${PLAY_MARK_SVG}</span>`;
// v0.17.7：文件不可访问标记（与正在播放 ♪ 同一位置、互斥；失效优先覆盖 ♪）。
const UNAVAILABLE_MARK_HTML = `<span class="unavailable-mark" title="文件不可访问（双击尝试恢复）" aria-label="文件不可访问">⚠</span>`;
// 当前播放曲目 id（queue 为空 / index 越界时为 null）。兼容 number / string。
function currentPlayingId() {
  const pb = state.playback;
  return pb && pb.queue ? (pb.queue[pb.index] ?? null) : null;
}
function isPlayingTrack(t) {
  const id = currentPlayingId();
  return id != null && String(t.local_id) === String(id);
}
// 切歌 / 清空队列 / 失效状态变化后增量更新列表标记：不重建表格，保留列宽 / 选中态 / 滚动位置。
// v0.17.7：同一位置管两种标记，失效（⚠）优先于正在播放（♪）——失效曲目此刻不可播放。
function syncTrackMarks() {
  document.querySelectorAll('.track-table .name-cell').forEach((cell) => {
    const id = cell.dataset.localId;
    const unavail = cell.dataset.unavailable === 'true';
    const play = cell.querySelector('.play-mark');
    const warn = cell.querySelector('.unavailable-mark');
    if (unavail) {
      if (play) play.remove();
      if (!warn) cell.insertAdjacentHTML('afterbegin', UNAVAILABLE_MARK_HTML);
    } else {
      if (warn) warn.remove();
      const should = isPlayingTrack({ local_id: id });
      if (should && !play) cell.insertAdjacentHTML('afterbegin', PLAY_MARK_HTML);
      else if (!should && play) play.remove();
    }
  });
}
// 兼容旧调用名（切歌等处沿用 syncPlayingMark）。
function syncPlayingMark() { syncTrackMarks(); }

// 歌曲列表列定义（key → 表头文案 + 单元格渲染）。顺序即默认显示顺序。
const COLUMN_DEFS = {
  name: {
    label: '曲名',
    render: (t) =>
      `<div class="name-cell" data-local-id="${t.local_id}" data-unavailable="${t.unavailable ? 'true' : 'false'}">` +
      (t.unavailable ? UNAVAILABLE_MARK_HTML : (isPlayingTrack(t) ? PLAY_MARK_HTML : '')) +
      `<span class="name-text">${esc(t.name)}</span>` +
      (t.has_lyrics ? '<span class="lyric-mark" title="有歌词">词</span>' : '') +
      `</div>`,
  },
  artist: { label: '艺人', render: (t) => esc(t.artist) },
  album: { label: '专辑', render: (t) => esc(t.album) },
  quality_tier: { label: '音质', render: (t) => esc(t.quality_tier) },
  duration_ms: { label: '时长', render: (t) => fmtTime(t.duration_ms) },
  file_size: { label: '文件大小', render: (t) => fmtSize(t.file_size) },
  imported_at: { label: '导入时间', render: (t) => fmtDateTimeCell(t.imported_at) },
  created_at: { label: '创建时间', render: (t) => fmtDateTimeCell(t.created_at) },
};
// 四张歌曲表（曲库 / 播放列表 / 专辑 / 歌手）共用同一套显示列与顺序，
// 「设置 → 列表 → 歌曲列表显示项」的勾选对它们同时生效。
function visibleTrackCols() {
  return state.columns.filter((k) => state.visible.has(k));
}
function trackHeadHtml(cols) {
  return cols.map((k) => `<th data-col-key="${k}">${esc(COLUMN_DEFS[k].label)}</th>`).join('');
}
function trackCellsHtml(t, cols) {
  return cols.map((k) => `<td>${COLUMN_DEFS[k].render(t)}</td>`).join('');
}
// 序号列内嵌收藏心形（不新增独立列，避免破坏列宽 / 拖拽 / nth-child 回归断言）。
// 四表共用：点击心形切换收藏，捕获阶段委托已阻止冒泡到行选中。
function idxCellHtml(t, i) {
  const fav = t.favorite ? '♥' : '♡';
  const cls = t.favorite ? 'fav-btn active' : 'fav-btn';
  return (
    `<td class="idx-col"><button class="${cls}" type="button" data-id="${t.local_id}" ` +
    `aria-label="收藏" aria-pressed="${!!t.favorite}">${fav}</button>` +
    `<span class="idx-num">${i + 1}</span></td>`
  );
}

// 四个页面（左侧导航入口）及其「纵向内联详情条」的 DOM 位置。
// 每个页面可独立选择横向（右侧独立详情栏）/ 纵向（详情内联到列表上方）。
const LAYOUT_VIEWS = {
  library:   { label: '全部曲库', wrap: 'lib-detail', cover: 'lib-detail-cover', fields: 'lib-detail-fields' },
  playlists: { label: '播放列表', wrap: 'pl-detail',  cover: 'pl-detail-cover',  fields: 'pl-detail-fields' },
  albums:    { label: '专辑',    wrap: 'al-detail',  cover: 'al-detail-cover',  fields: 'al-detail-fields' },
  artists:   { label: '歌手',    wrap: 'ar-detail',  cover: 'ar-detail-cover',  fields: 'ar-detail-fields' },
};
const VIEW_KEYS = Object.keys(LAYOUT_VIEWS);

// 每个页面的「歌曲列表容器」id（用于定位表格、同步选中高亮）。
const VIEW_BOX = {
  library: 'library-scroll',
  playlists: 'pl-content',
  albums: 'al-content',
  artists: 'ar-content',
  // #32：收藏复用曲库表格，故共用同一个滚动容器（无二级栏）。
  favorites: 'library-scroll',
  // v0.12.7：曲源页三级歌曲表容器，供选中高亮 / 右键菜单定位。
  sources: 'sv-content',
};

// 布局风格的显示文案（1 = 横向：详情在右侧独立栏；2 = 纵向：详情内联到列表上方）。
const LAYOUT_STYLE_LABELS = { 1: '横向', 2: '纵向' };

// 列宽占比下限（%）：与后端 config::MIN_COLUMN_WIDTH_PCT 保持一致。
const MIN_COL_PCT = 4;
// 列宽像素下限（px）：单元格左右内边距合计 24px，再留一点文字宽度；
// 窄于此值浏览器会强制撑宽（border-box 下 padding 不可压缩），导致表格溢出。
const MIN_COL_PX = 48;
// 八列默认宽度占比（%），合计 100；与后端 config.rs 的 default_width_* 保持一致。
const DEFAULT_COL_WIDTHS = {
  name: 24, artist: 16, album: 18, quality_tier: 6,
  duration_ms: 6, file_size: 8, imported_at: 11, created_at: 11,
};

async function api(path, opts) {
  // v0.12.7：可选超时（仅轮询类请求传入）。超时只结束前端等待，
  // 不 abort 后端正在执行的导入；导入请求本身不传 timeoutMs，保持长连接语义。
  const timeoutMs = opts && opts.timeoutMs ? opts.timeoutMs : 0;
  const init = Object.assign({}, opts);
  delete init.timeoutMs;
  let ctrl = null;
  let timer = null;
  if (timeoutMs > 0 && typeof AbortController !== 'undefined') {
    ctrl = new AbortController();
    init.signal = ctrl.signal;
    timer = setTimeout(() => ctrl.abort(), timeoutMs);
  }
  try {
    const res = await fetch(path, init);
    let data = null;
    try { data = await res.json(); } catch (e) { /* 空体 */ }
    return { ok: res.ok, data };
  } catch (e) {
    return { ok: false, data: null };
  } finally {
    if (timer) clearTimeout(timer);
  }
}

// v0.12.7：字典取值的统一入口（旧 dist 缺语言文件时退回 fallback，不外抛）。
// 集中在此处引用 key，便于 check_i18n.py 校验「字典无未使用条目」。
function t(key, params) {
  return (window.KQK && typeof window.KQK.t === 'function')
    ? window.KQK.t(key, params || {}) : ((params && params._fallback) || '');
}

function fmtTime(ms) {
  if (!ms || ms < 0) ms = 0;
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const r = s % 60;
  return m + ':' + String(r).padStart(2, '0');
}

// ---------- 时间戳显示（导入时间 / 创建时间）----------
// 后端统一给 UNIX **毫秒**（见 db.rs TrackView），用 Date 直接按本地时区渲染，避免
// SQLite datetime 文本（UTC、无时区标记）在浏览器里被误解析成 UTC 或本地时间。
// 同年显示「MM-DD HH:MM」节省列宽，跨年显示「YYYY-MM-DD」；完整值放 title 供 hover 查看。
function fmtDateTimeParts(ms) {
  const n = Number(ms);
  if (!isFinite(n) || n <= 0) return null;
  const d = new Date(n);
  if (isNaN(d.getTime())) return null;
  const p = (x) => String(x).padStart(2, '0');
  const y = d.getFullYear();
  const md = p(d.getMonth() + 1) + '-' + p(d.getDate());
  const hm = p(d.getHours()) + ':' + p(d.getMinutes());
  return {
    full: y + '-' + md + ' ' + hm + ':' + p(d.getSeconds()),
    short: (y === new Date().getFullYear()) ? (md + ' ' + hm) : (y + '-' + md),
  };
}

function fmtDateTimeFull(ms) {
  const p = fmtDateTimeParts(ms);
  return p ? p.full : '';
}

// 列表单元格：空值返回空串（**不渲染「—」等占位内容**，遵守详情区隐藏契约）。
function fmtDateTimeCell(ms) {
  const p = fmtDateTimeParts(ms);
  if (!p) return '';
  return `<span class="dt-cell" title="${esc(p.full)}">${esc(p.short)}</span>`;
}

// ---------- 导入进度面板 ----------
// 后端导入在 blocking 线程执行（HTTP 请求直到全部完成才返回），
// 因此这里在发起导入请求的同时轮询 /api/import-progress 拉取实时快照。
// 轮询在导入请求返回后停止，导入结果仍以请求返回值为准。

const progressState = {
  timer: null,
  // 日志游标：已渲染到的 seq（后端日志「新的在前」，靠 seq 精确取增量，不重不漏）。
  logSeq: 0,
  visible: false,
  // 空积压单行模式：renderProgress 直接 return，避免残留轮询帧覆盖「无积压」提示。
  emptyMode: false,
  // 自动消失定时器（空积压 + 正常完成场景共用）。
  autoHideTimer: null,
  // v0.14.5：任务是否已结算完成（决定完成态收口：标题/按钮/spinner/自动消失）。
  finished: false,
  // v0.14.5：用户是否已点「×」彻底关闭（此类不再自动弹面板/胶囊，但后台任务继续）。
  dismissed: false,
  // v0.12.18：暂停=POST 后端真实挂起导入线程；停止是协作式取消，不回滚已入库。
  paused: false,
  cancelled: false,
  taskActive: false,
  // 轮询连续失败次数 + 最近一次快照（面板隐藏时供胶囊读取）。
  failStreak: 0,
  lastSnapshot: null,
  // 胶囊状态：updating / paused / stopped / done / error。
  pillMode: 'updating',
  // v0.15.1：任务种类（import=导入 / reorg=曲库重组）与重组计时。
  kind: 'import',
  startedAt: 0,
  pausedAt: 0,
  pausedMs: 0,
  // v0.17.1：导入期间「新入库曲目」增量游标与待刷新缓冲。
  // importSeq 按后端 new_imports[].seq 增量消费；pendingImports 攒够一批（约 10 首）
  // 才追加进 state.tracks 并渲染一次，避免逐首重绘。
  importSeq: 0,
  pendingImports: [],
};

/** 秒 → mm:ss（超过 1 小时显示 hh:mm:ss，用于重组已用 / 预计剩余时间）。 */
function fmtProgressDuration(sec) {
  const s = Math.max(0, Math.floor(sec || 0));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const mm = String(m).padStart(2, '0');
  const ss = String(r).padStart(2, '0');
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

// 动作 key → 中文文案（与后端 progress.rs 的 ImportAction 对应）。
const ACTION_LABELS = {
  copy: '复制',
  move: '移动',
  convert: '转换',
  stage: '暂存',
  skip: '跳过',
};
const STATUS_LABELS = {
  imported: '已入库',
  converted: '已转换',
  skipped: '已跳过',
  held: '重复挂起',
  failed: '失败',
};

function showProgressPanel(title, kind) {
  state.progressTitle = title || '正在处理…';
  const panel = $('progress-panel');
  if (!panel) return;
  clearAutoHideProgressPanel();
  progressState.emptyMode = false;
  // v0.15.1：新一轮任务重置种类与计时；重组据此显示时间行与「结束（回滚）」文案。
  progressState.kind = kind === 'reorg' ? 'reorg' : 'import';
  progressState.startedAt = Date.now();
  progressState.pausedAt = 0;
  progressState.pausedMs = 0;
  const stopBtn = $('pp-stop');
  if (stopBtn) {
    stopBtn.textContent = progressState.kind === 'reorg' ? '结束' : t('progress.stop', { _fallback: '停止' });
    stopBtn.title = progressState.kind === 'reorg' ? '结束并回滚到原结构' : '停止导入';
    stopBtn.setAttribute('aria-label', stopBtn.title);
  }
  // v0.14.5：新一轮任务重置完成态 / 关闭态。
  progressState.finished = false;
  progressState.dismissed = false;
  // v0.12.7：新一轮任务重置暂停 / 取消 / 失败计数，收起胶囊。
  progressState.paused = false;
  progressState.cancelled = false;
  progressState.failStreak = 0;
  progressState.taskActive = true;
  progressState.lastSnapshot = null;
  // v0.17.1：新一轮任务重置导入明细游标与缓冲。
  progressState.importSeq = 0;
  progressState.pendingImports = [];
  setProgressPillMode('updating');
  hideProgressPill();
  syncProgressButtons();
  panel.hidden = false;
  progressState.visible = true;
  progressState.logSeq = 0;
  $('pp-title').textContent = state.progressTitle;
  $('pp-count').textContent = '';
  // 复位空积压模式隐藏的子元素与进度条状态。
  const bar = panel.querySelector('.pp-bar');
  if (bar) { bar.hidden = false; bar.classList.remove('indeterminate'); }
  const log = $('pp-log');
  if (log) { log.hidden = false; log.innerHTML = ''; }
  const cur = $('pp-current');
  if (cur) cur.hidden = false;
  const spin = panel.querySelector('.pp-spinner');
  if (spin) spin.hidden = false;
  $('pp-current').textContent = '';
  // v0.17.2：两行分类计数复位。
  const split = $('pp-split');
  if (split) split.hidden = false;
  const convertedEl = $('pp-converted');
  if (convertedEl) convertedEl.textContent = '0';
  const importedEl = $('pp-imported');
  if (importedEl) importedEl.textContent = '0';
  const timeEl = $('pp-time');
  if (timeEl) timeEl.textContent = '';
  $('pp-summary').textContent = '';
  $('pp-summary').hidden = false;
  const fill = $('pp-bar-fill');
  if (fill) fill.style.width = '0%';
}

// 空积压 / 无任务：显示单行提示，隐藏留白进度条与日志，随后由 scheduleAutoHideProgressPanel 自动消失。
function showProgressEmpty(line) {
  const panel = $('progress-panel');
  if (!panel) return;
  progressState.emptyMode = true;
  $('pp-title').textContent = state.progressTitle || '处理完成';
  const bar = panel.querySelector('.pp-bar');
  if (bar) { bar.hidden = true; bar.classList.remove('indeterminate'); }
  const log = $('pp-log');
  if (log) { log.hidden = true; log.innerHTML = ''; }
  const cur = $('pp-current');
  if (cur) { cur.hidden = true; cur.textContent = ''; }
  const split = $('pp-split');
  if (split) split.hidden = true;
  const spin = panel.querySelector('.pp-spinner');
  if (spin) spin.hidden = true;
  $('pp-count').textContent = '';
  const sum = $('pp-summary');
  if (sum) { sum.hidden = false; sum.textContent = line; }
}

// 几秒后自动收起进度面板（空积压场景用）；重复调用不会叠加定时器。
function scheduleAutoHideProgressPanel(ms) {
  clearAutoHideProgressPanel();
  progressState.autoHideTimer = setTimeout(() => {
    progressState.autoHideTimer = null;
    progressState.emptyMode = false;
    finishProgressPanel();
  }, ms);
}

function clearAutoHideProgressPanel() {
  if (progressState.autoHideTimer) {
    clearTimeout(progressState.autoHideTimer);
    progressState.autoHideTimer = null;
  }
}

// 启动进度轮询（每 400ms 拉一次）。幂等：重复调用不会起多个定时器。
function startProgressPolling() {
  stopProgressPolling();
  progressState.taskActive = true;
  progressState.timer = setInterval(pollProgress, 400);
}

function stopProgressPolling() {
  if (progressState.timer) {
    clearInterval(progressState.timer);
    progressState.timer = null;
  }
}

async function pollProgress() {
  // v0.12.7：轮询请求带 3s 超时；连续失败 3 次才降级提示，避免偶发抖动刷屏。
  const { ok, data } = await api('/api/import-progress', { timeoutMs: 3000 });
  if (!ok || !data) {
    progressState.failStreak += 1;
    if (progressState.failStreak === 3) {
      setProgressPillMode('error');
      setHint(t('progress.conn_error', { _fallback: '连接异常，进度刷新暂不可用' }));
    }
    return;
  }
  progressState.failStreak = 0;
  if (progressState.pillMode === 'error') setProgressPillMode('updating');
  progressState.lastSnapshot = data;
  // 后端已收敛（processed 到达 total）→ 胶囊切「已完成」。
  // v0.12.18：以快照的 paused 为真值同步前端暂停态（后端才是暂停的唯一事实源）。
  progressState.paused = !!data.paused;
  if (data.cancelled) setProgressPillMode('stopped');
  else if (data.paused) setProgressPillMode('paused');
  else if (data.total > 0 && data.processed >= data.total && !data.running) setProgressPillMode('done');
  else if (progressState.pillMode === 'paused') setProgressPillMode('updating');
  renderProgress(data);
  consumeImportTracks(data);
  syncProgressButtons();
}

/** v0.17.2：导入期间把后端 new_imports 增量按 seq 攒入缓冲，每约 10 首追加进
 *  全部列表并渲染一次；收敛时 flush 剩余。v0.17.2 两处修复：① 去掉 state.activeTab==='library'
 *  门槛（曲源页 / 向导导入也照常累积，否则导入期间列表不刷新）；② 新增 force 参数，供
 *  finishProgressTask 在轮询停止后用最终快照强制刷出尾批（不足 10 首时原逻辑永远丢失）。
 *  重组任务不处理。 */
function consumeImportTracks(p, force) {
  if (progressState.kind !== 'reorg' && Array.isArray(p.new_imports)) {
    const fresh = p.new_imports.filter((t) => (t.seq || 0) > progressState.importSeq);
    if (fresh.length) {
      fresh.sort((a, b) => (a.seq || 0) - (b.seq || 0));
      for (const t of fresh) progressState.importSeq = Math.max(progressState.importSeq, t.seq || 0);
      progressState.pendingImports.push(...fresh);
    }
    // 每攒够 10 首刷一次；导入已收敛或 finish 强制时把剩余全部刷出（不遗漏尾批）。
    const settled = !!force || (!p.running && p.total > 0 && p.processed >= p.total);
    if (progressState.pendingImports.length >= 10 || (settled && progressState.pendingImports.length)) {
      for (const t of progressState.pendingImports) {
        if (!state.tracks.some((x) => Number(x.local_id) === Number(t.local_id))) state.tracks.push(t);
      }
      progressState.pendingImports = [];
      renderTracks();
    }
  }
}

function renderProgress(p) {
  const panel = $('progress-panel');
  if (!panel) return;
  // v0.12.18：暂停时面板保持展开并继续显示真实进度（不再只在胶囊显示）；
  // 仅「面板收起」时才跳过面板 DOM，仅更新胶囊。
  if (!progressState.visible) {
    // v0.14.5：用户点「×」彻底关闭后，后台轮询更新也不再弹胶囊。
    if (!progressState.dismissed) renderProgressPill(p);
    return;
  }
  // 空积压单行模式：残留轮询帧不要覆盖「无积压」提示。
  if (progressState.emptyMode) return;

  // 标题：本次任务来源 + 真实状态。
  // v0.14.5：完成态用「已完成」收口（不再停在「导入中」）；v0.12.18 的暂停/停止态保留。
  const title = $('pp-title');
  if (title && p.source) {
    if (progressState.kind === 'reorg') {
      // 重组阶段文案由后端 set_source 下发（扫描 / 移动 / 清理 / 回滚 / 完成），
      // 不再拼「导入中」；暂停单独标注，完成/取消状态已含在后端文案里。
      title.textContent = p.source + (p.paused && !p.cancelled && !progressState.finished ? '（已暂停）' : '');
    } else {
      // 后端已收敛（running=false 且 processed 达 total）也视为完成，避免异步快照稍慢时
      // 面板停在「导入中」——这是「看起来已完成却还显示更新中」的直接修复。
      const backendDone = !p.running && p.total > 0 && p.processed >= p.total;
      const phase = p.cancelled
        ? t('progress.stopped', { _fallback: '已停止' })
        : (progressState.finished || backendDone)
          ? t('progress.done', { _fallback: '已完成' })
          : p.paused
            ? t('progress.paused', { _fallback: '已暂停' })
            : t('progress.updating', { _fallback: '导入中' });
      title.textContent = p.source + '：' + phase;
    }
  }

  // 计数：已处理 / 总数。
  const count = $('pp-count');
  if (count) {
    count.textContent = p.total > 0 ? `${p.processed} / ${p.total}` : `${p.processed}`;
  }

  // v0.17.2：转换 / 导入两行分类计数（后端 converted 为结果计数，imported 为入库数）。
  // 重组（kind=reorg）不是导入流程，隐藏这两行。
  const split = $('pp-split');
  if (split) {
    split.hidden = progressState.kind === 'reorg';
    const c = $('pp-converted');
    if (c) c.textContent = String(Number(p.converted) || 0);
    const imp = $('pp-imported');
    if (imp) imp.textContent = String(Number(p.imported) || 0);
  }

  // 进度条：总数未知（扫描阶段）时用不确定态（CSS 动画）。
  const fill = $('pp-bar-fill');
  if (fill) {
    if (p.total > 0) {
      const pct = Math.min(100, Math.round((p.processed / p.total) * 100));
      fill.style.width = pct + '%';
      fill.parentElement.classList.remove('indeterminate');
    } else {
      fill.style.width = '30%';
      fill.parentElement.classList.add('indeterminate');
    }
  }

  // 当前处理：文件名 + 当前动作（复制/移动/转换/暂存）。
  const cur = $('pp-current');
  if (cur) {
    const verb = p.current_action ? (ACTION_LABELS[p.current_action] || p.current_action) : '';
    if (p.current_file) {
      cur.textContent = verb ? `${verb}：${p.current_file}` : p.current_file;
      cur.title = cur.textContent;
    } else {
      cur.textContent = '';
      cur.title = '';
    }
  }

  // v0.15.1：重组显示已用 / 预计剩余时间（线性外推；暂停期间不计入已用）。
  const timeEl = $('pp-time');
  if (timeEl) {
    if (progressState.kind === 'reorg' && progressState.startedAt) {
      const now = Date.now();
      if (p.paused && !progressState.pausedAt) progressState.pausedAt = now;
      if (!p.paused && progressState.pausedAt) {
        progressState.pausedMs += now - progressState.pausedAt;
        progressState.pausedAt = 0;
      }
      const pausedNow = progressState.pausedAt ? now - progressState.pausedAt : 0;
      const elapsedMs = Math.max(0, now - progressState.startedAt - progressState.pausedMs - pausedNow);
      const elapsed = Math.floor(elapsedMs / 1000);
      let txt = (progressState.finished ? '用时 ' : '已用 ') + fmtProgressDuration(elapsed);
      if (p.paused && !progressState.finished) txt += ' · 预计剩余 —（已暂停）';
      else if (progressState.finished || p.cancelled) { /* 完成 / 结束不再显示剩余 */ }
      else if (p.total > 0 && p.processed > 0) {
        const remain = Math.ceil((p.total - p.processed) * elapsedMs / p.processed / 1000);
        txt += ' · 预计剩余 ' + fmtProgressDuration(remain);
      } else txt += ' · 预计剩余 —';
      timeEl.textContent = txt;
    } else {
      timeEl.textContent = '';
    }
  }

  // 动作日志：后端 recent 为「新的在前」，按 seq 游标取增量（不重不漏）。
  const log = $('pp-log');
  if (log) {
    const fresh = (p.recent || []).filter((e) => (e.seq || 0) > progressState.logSeq);
    if (fresh.length) {
      // fresh 是新→旧，反向插入使列表始终「旧在上、新在下」（符合阅读习惯）。
      for (let i = fresh.length - 1; i >= 0; i--) {
        const e = fresh[i];
        progressState.logSeq = Math.max(progressState.logSeq, e.seq || 0);
        const li = document.createElement('li');
        li.className = 'pp-log-item st-' + e.status;
        const verb = ACTION_LABELS[e.action] || e.action;
        const st = STATUS_LABELS[e.status] || e.status;
        li.innerHTML = `<span class="pp-log-verb">${esc(verb)}</span>` +
          `<span class="pp-log-name" title="${esc(e.name)}">${esc(e.name)}</span>` +
          `<span class="pp-log-status">${esc(st)}</span>`;
        if (e.message) {
          const m = document.createElement('span');
          m.className = 'pp-log-msg';
          m.textContent = '（' + e.message + '）';
          li.appendChild(m);
        }
        log.insertBefore(li, log.firstChild);
        // 只保留最近 60 条 DOM 节点，避免长时间导入堆积。
        while (log.children.length > 60) log.removeChild(log.lastChild);
      }
      log.scrollTop = log.scrollHeight;
    }
  }

  // 底部汇总计数；取消后切换为「已停止，保留已入库」。
  // v0.14.5：补「重复挂起」精确计数，使 imported+skipped+pending+held+failed == processed。
  const sum = $('pp-summary');
  if (sum && progressState.kind === 'reorg') {
    sum.textContent = p.cancelled
      ? '已结束并回滚到原结构'
      : (progressState.finished || (!p.running && p.total > 0 && p.processed >= p.total))
        ? `重组完成：已移动 ${p.processed} 首`
        : `正在重组：已处理 ${p.processed}${p.total ? ' / ' + p.total : ''} 首`;
    sum.classList.toggle('is-stopped', !!p.cancelled);
  } else if (sum) {
    const parts = [`入库 ${p.imported}`, `跳过 ${p.skipped}`];
    if (p.held) parts.push(`重复挂起 ${p.held}`);
    if (p.failed) parts.push(`失败 ${p.failed}`);
    sum.textContent = p.cancelled
      ? t('progress.stopped_keep', { _fallback: '已停止，保留已入库' }) + '，' + parts.join('，')
      : parts.join('，');
    sum.classList.toggle('is-stopped', !!p.cancelled);
  }
}

// ---------- v0.12.7 胶囊 / 暂停 / 停止 ----------
function progressPillLabel(mode) {
  if (mode === 'paused') return t('progress.paused', { _fallback: '已暂停' });
  if (mode === 'stopped') return progressState.kind === 'reorg' ? '已结束' : t('progress.stopped', { _fallback: '已停止' });
  if (mode === 'done') return t('progress.done', { _fallback: '已完成' });
  if (mode === 'error') return t('progress.conn_error', { _fallback: '连接异常' });
  return progressState.kind === 'reorg' ? '重组中' : t('progress.updating', { _fallback: '导入中' });
}

function setProgressPillMode(mode) {
  progressState.pillMode = mode;
  const pill = $('progress-pill');
  if (pill) pill.className = 'progress-pill st-' + mode;
  const status = $('pill-status');
  if (status) status.textContent = progressPillLabel(mode);
  const dot = $('pill-dot');
  if (dot) dot.className = 'pill-dot';
}

function renderProgressPill(p) {
  const count = $('pill-count');
  if (count) count.textContent = p && p.total > 0 ? `${p.processed} / ${p.total}` : `${(p && p.processed) || 0}`;
  if (p && p.cancelled) setProgressPillMode('stopped');
  showProgressPill();
}

function showProgressPill() {
  const pill = $('progress-pill');
  if (pill) pill.hidden = false;
}

function hideProgressPill() {
  const pill = $('progress-pill');
  if (pill) pill.hidden = true;
}

/** 按当前状态同步「暂停/继续」「停止」「最小化/关闭」按钮文案、可用态。 */
function syncProgressButtons() {
  // v0.14.5：完成/取消/未激活后禁用暂停与停止（消除「点了没反应」的误导）。
  const dead = progressState.finished || progressState.cancelled || !progressState.taskActive;
  const pause = $('pp-pause');
  if (pause) {
    pause.textContent = progressState.paused
      ? t('progress.resume', { _fallback: '继续' })
      : t('progress.pause', { _fallback: '暂停' });
    pause.classList.toggle('is-active', progressState.paused);
    pause.disabled = dead;
  }
  const stop = $('pp-stop');
  if (stop) stop.disabled = dead || !!progressState.cancelled;
}

/** 收起：保留轮询（后台继续）、只显示胶囊；任务结束才真正停轮询。 */
function collapseProgressPanel() {
  const panel = $('progress-panel');
  if (panel) panel.hidden = true;
  progressState.visible = false;
  renderProgressPill(progressState.lastSnapshot);
}

/** 「×」彻底关闭：隐藏面板与胶囊且不再弹出；后台导入继续，不碰轮询/taskActive。 */
function dismissProgressPanel() {
  const panel = $('progress-panel');
  if (panel) panel.hidden = true;
  progressState.visible = false;
  progressState.dismissed = true;
  clearAutoHideProgressPanel();
  hideProgressPill();
}

/** 完成态收口（v0.14.5）：渲染最终帧、标题「已完成」、按钮禁用、停 spinner、胶囊 done，
 *  并安排约 2.5s 后自动消失。幂等：重复调用不再叠加定时器。 */
function settleProgressDone() {
  if (progressState.finished) return;
  progressState.finished = true;
  progressState.paused = false;
  const final = progressState.lastSnapshot;
  // 面板可见且非空积压：用最终快照刷新（标题走「已完成」、计数补上重复挂起）。
  if (final && progressState.visible && !progressState.emptyMode) renderProgress(final);
  setProgressPillMode(progressState.cancelled ? 'stopped' : 'done');
  if (!progressState.visible && !progressState.dismissed) showProgressPill();
  syncProgressButtons();
  const panel = $('progress-panel');
  const spin = panel && panel.querySelector('.pp-spinner');
  if (spin) spin.hidden = true;
  // 完成 / 停止后自动消失；空积压场景由 showProgressEmpty 的 schedule 覆盖为 3s。
  if (!progressState.dismissed) scheduleAutoHideProgressPanel(2500);
}

/** 任务结束：停轮询、隐藏面板与胶囊（自动消失定时器用）。 */
function finishProgressPanel() {
  stopProgressPolling();
  progressState.taskActive = false;
  clearAutoHideProgressPanel();
  progressState.emptyMode = false;
  const panel = $('progress-panel');
  if (panel) panel.hidden = true;
  progressState.visible = false;
  hideProgressPill();
}

/** 导入请求返回：主动拉一次最终快照（此刻后端 running=false、计数已结算），
 *  解决「面板停在倒数第二帧（入库 20 vs 主界面 23）」，随后完成态收口 + 自动消失。 */
function finishProgressTask() {
  stopProgressPolling();
  progressState.taskActive = false;
  progressState.failStreak = 0;
  api('/api/import-progress', { timeoutMs: 3000 }).then(({ ok, data }) => {
    if (ok && data) {
      progressState.lastSnapshot = data;
      progressState.cancelled = !!data.cancelled;
      // v0.17.2 A：轮询已停，这里用最终快照强制刷出不足 10 首的尾批 new_imports。
      consumeImportTracks(data, true);
    }
    // v0.17.2 B：导入完成后无条件重载权威列表，确保增量链路失效时列表也不为空。
    if (progressState.kind !== 'reorg') loadTracks($('search').value);
    settleProgressDone();
  }).catch(() => {
    if (progressState.kind !== 'reorg') loadTracks($('search').value);
    settleProgressDone();
  });
}

/** 面板隐藏后重新展开：恢复轮询；暂停态以后端快照为准，不在此擅自清除。 */
function expandProgressPanel() {
  const panel = $('progress-panel');
  if (panel) panel.hidden = false;
  progressState.visible = true;
  progressState.dismissed = false;
  clearAutoHideProgressPanel();
  setProgressPillMode(progressState.cancelled ? 'stopped'
    : (progressState.paused ? 'paused'
      : (progressState.taskActive ? 'updating' : (progressState.pillMode === 'error' ? 'error' : 'done'))));
  hideProgressPill();
  syncProgressButtons();
  if (progressState.taskActive) startProgressPolling();
  if (progressState.lastSnapshot) renderProgress(progressState.lastSnapshot);
}

/** 暂停 / 继续：POST 后端真实挂起/恢复导入；暂停时面板保持展开并继续轮询，进度自然停在最新帧。 */
async function toggleProgressPause() {
  if (!progressState.taskActive) return;
  const toPaused = !progressState.paused;
  const endpoint = toPaused ? '/api/import-pause' : '/api/import-resume';
  const { ok } = await api(endpoint, { method: 'POST', timeoutMs: 3000 });
  if (!ok) {
    setHint(t('progress.conn_error', { _fallback: '连接异常，暂停请求未生效' }));
    return;
  }
  progressState.paused = toPaused;
  progressState.visible = true;
  const panel = $('progress-panel');
  if (panel) panel.hidden = false;
  hideProgressPill();
  setProgressPillMode(toPaused ? 'paused' : 'updating');
  syncProgressButtons();
  // 暂停后不停止轮询：后端挂起进度自然不动；继续后立即恢复真实刷新。
  if (!progressState.timer && progressState.taskActive) startProgressPolling();
  // 用刚请求的暂停态覆盖快照再渲染，让标题/状态即时反映，不等下一次轮询帧。
  if (progressState.lastSnapshot) {
    renderProgress(Object.assign({}, progressState.lastSnapshot, { paused: toPaused }));
  }
}

/** 停止：POST 取消标志。导入=保留已入库；重组=整体回滚到原结构。 */
async function stopProgressTask() {
  if (!progressState.taskActive || progressState.cancelled) return;
  progressState.cancelled = true;
  setProgressPillMode('stopped');
  syncProgressButtons();
  const { ok } = await api('/api/import-cancel', { method: 'POST', timeoutMs: 3000 });
  if (!ok) setHint(t('progress.conn_error', { _fallback: '连接异常，进度刷新暂不可用' }));
  // 保持轮询到后端收敛，胶囊持续显示 N/M 与「已停止」。
  if (!progressState.timer && progressState.taskActive) startProgressPolling();
}

// v0.14.5：最小化=收起为胶囊（可点胶囊恢复）；×=彻底关闭（不再弹，后台继续）。
const ppMinimize = $('pp-minimize');
if (ppMinimize) ppMinimize.addEventListener('click', collapseProgressPanel);
const ppClose = $('pp-close');
if (ppClose) ppClose.addEventListener('click', dismissProgressPanel);
const ppPause = $('pp-pause');
if (ppPause) ppPause.addEventListener('click', toggleProgressPause);
const ppStop = $('pp-stop');
if (ppStop) ppStop.addEventListener('click', stopProgressTask);
const progressPill = $('progress-pill');
if (progressPill) progressPill.addEventListener('click', expandProgressPanel);

// 文件大小格式化（字节 → 易读单位）。
function fmtSize(bytes) {
  if (!bytes || bytes < 0) bytes = 0;
  if (bytes < 1024) return bytes + ' B';
  const kb = bytes / 1024;
  if (kb < 1024) return kb.toFixed(1) + ' KB';
  const mb = kb / 1024;
  if (mb < 1024) return mb.toFixed(1) + ' MB';
  return (mb / 1024).toFixed(2) + ' GB';
}

function esc(s) {
  return String(s == null ? '' : s).replace(/[&<>"]/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c])
  );
}

let hintHideTimer = null;
function hideHint() {
  if (hintHideTimer) { clearTimeout(hintHideTimer); hintHideTimer = null; }
  $('hint').textContent = '';
  const wrap = $('hint-wrap');
  if (wrap) wrap.hidden = true;
}
/** 成功反馈可传 autoHideMs 定时收起；错误 / 失败类不传，保持常驻。 */
function setHint(msg, opts) {
  if (hintHideTimer) { clearTimeout(hintHideTimer); hintHideTimer = null; }
  $('hint').textContent = msg || '';
  const wrap = $('hint-wrap');
  if (wrap) wrap.hidden = !msg;
  const ms = opts && opts.autoHideMs;
  if (msg && ms > 0) {
    hintHideTimer = setTimeout(() => { hintHideTimer = null; hideHint(); }, ms);
  }
}
/** 纯成功确认提示：5s 后自动收起（可传 ms 覆盖）；错误 / 警告 / 进度仍用 setHint 常驻。 */
function setHintOk(msg, ms = 5000) { setHint(msg, { autoHideMs: ms }); }
$('hint-close').addEventListener('click', hideHint);

// ---------- 状态与界面 ----------
async function refreshStatus() {
  const { data } = await api('/api/status');
  if (data) {
    state.library_configured = !!data.library_configured;
    state.library_dir = data.library_dir || '';
    state.library_name = data.library_name || '';
    state.netease_library_dir = data.netease_library_dir || '';
    state.file_scheme = data.file_scheme || 'artist_album';
    state.ncm_ready = !!data.ncm_ready;
    state.ncm_enabled = data.ncm_enabled !== false;
    state.ncm_tool = data.ncm_tool || '';
    state.pendingDuplicates = Number(data.pending_duplicates) || 0;
    state.duplicatesDir = data.pending_duplicates_dir || '';
    updateDupBadge(state.pendingDuplicates);
    // 配置文件版本状态（生成版本 / 最低兼容版本 / 兼容判定 / 冲突原因）。
    state.config_version = data.config_version || '';
    state.config_min_version = data.config_min_compatible_version || '';
    state.config_compat = data.config_compat || '';
    state.config_conflict = data.config_conflict_reason || '';
    state.library_config_version = data.library_config_version || '';
    state.library_config_compat = data.library_config_compat || '';
    state.library_config_conflict = data.library_config_conflict_reason || '';
    state.app_version = data.app_version || '';
    // v0.14.13：窗口最大化态由 GUI 壳写入权威值，首屏 / 设置重读时校准图标。
    if (typeof data.maximized === 'boolean') applyWindowMaximizeState(data.maximized);
    // #34：左侧品牌区第二行显示编译期版本号，与窗口标题 / 设置·关于同源，升级版本自动跟随。
    const brandVer = $('brand-version');
    if (brandVer) brandVer.textContent = state.app_version ? 'v' + state.app_version : '';
    // FR-30：联网四键。后端 sanitize() 已把枚举钳到已登记值，这里再兜一层：
    // 未下发（老版本服务）一律保持「全部关闭」，绝不因字段缺失而隐式联网。
    state.networkEnabled = data.network_enabled === true;
    state.updateCheckEnabled = data.update_check_enabled === true;
    // 当前仅登记 GitHub 一个更新源；未登记值一律回落 github。
    state.updateSource = data.update_source === 'github' ? 'github' : 'github';
    state.updateInterval = ['daily', 'weekly', 'monthly'].includes(data.update_check_interval)
      ? data.update_check_interval : 'daily';
    // 外观 / 主题方案：以配置文件为准（或把旧版 localStorage 偏好一次性迁移进配置）。
    syncAppearanceFromStatus(data.appearance);
    // 自动播放列表上限 n（app 级设置，换曲库不变）。
    if (data.auto_playlist_count !== undefined) {
      state.autoPlaylistCount = clampAutoCount(data.auto_playlist_count);
    }
    // 歌曲筛选阈值（app 级 [import]，坏值由后端 sanitize；这里再兜一层）。
    if (data.min_size_kb !== undefined) {
      const n = Number(data.min_size_kb);
      state.importPrefs.minSizeKb = Number.isFinite(n) && n >= 0 && n <= 102400 ? Math.round(n) : 500;
    }
    if (data.min_duration_sec !== undefined) {
      const n = Number(data.min_duration_sec);
      state.importPrefs.minDurationSec = Number.isFinite(n) && n >= 0 && n <= 3600 ? Math.round(n) : 30;
    }
    if (data.auto_update_sources_on_start !== undefined) {
      state.importPrefs.autoUpdateSourcesOnStart = !!data.auto_update_sources_on_start;
    }
    renderImportPrefs();
    // 解析歌曲列表显示列配置（持久化的是可见列有序列表；补齐缺失列并保留顺序）。
    // 后端 UiConfig::sanitize() 已做过旧配置迁移，这里再兜一层：
    // 若拿到的是完全不含时间列的旧配置（例如后端尚未重启 / 老版本服务），
    // 只自动补「导入时间」，**不补「创建时间」**（创建时间默认隐藏，归设置面板管）。
    if (Array.isArray(data.track_columns) && data.track_columns.length) {
      const saved = data.track_columns.filter((k) => ALL_COLUMN_KEYS.includes(k));
      if (!data.track_columns_saved && !saved.some((k) => k === 'imported_at' || k === 'created_at')) {
        saved.push('imported_at');
      }
      const savedSet = new Set(saved);
      state.columns = saved.concat(ALL_COLUMN_KEYS.filter((k) => !savedSet.has(k)));
      state.visible = savedSet;
    } else {
      state.columns = ALL_COLUMN_KEYS.slice();
      state.visible = new Set(DEFAULT_VISIBLE_COLUMN_KEYS);
    }
    // 解析布局风格（各页面独立：1 = 横向/右侧详情栏，2 = 纵向/详情内联到列表上方）。
    if (data.layout_styles && typeof data.layout_styles === 'object') {
      VIEW_KEYS.forEach((v) => {
        const s = Number(data.layout_styles[v]);
        if (s === 1 || s === 2) state.layoutStyles[v] = s;
      });
    }
    // 解析列宽比例（%）：非法值忽略，保留默认值。
    if (data.column_widths && typeof data.column_widths === 'object') {
      ALL_COLUMN_KEYS.forEach((k) => {
        const w = Number(data.column_widths[k]);
        if (isFinite(w) && w > 0) state.columnWidths[k] = w;
      });
    }
    // FR-27：detail_collapsed 必须在布局仲裁之前读回，syncDetailVisibility() 靠它决定
    // 收起态详情栏 / 内联窄条是否常驻（否则已持久化的收起态会在启动时被误隐藏）。
    if (typeof data.detail_collapsed === 'boolean') state.detailCollapsed = data.detail_collapsed;
    applyLayoutStyle();
    // FR-24：读回侧栏宽度 / 收缩态 / 音量（app 级 [ui]，坏值由后端 sanitize 挡掉，这里再兜一层）。
    if (data.sidebar_width !== undefined) state.sidebarWidth = clampPaneWidth(Number(data.sidebar_width), 'sidebar');
    if (data.group_width !== undefined) state.groupWidth = clampPaneWidth(Number(data.group_width), 'group');
    if (typeof data.sidebar_collapsed === 'boolean') state.sidebarCollapsed = data.sidebar_collapsed;
    if (typeof data.group_collapsed === 'boolean') state.groupCollapsed = data.group_collapsed;
    if (data.volume !== undefined) state.volume = Math.min(100, Math.max(0, Number(data.volume) || 0));
    if (typeof data.muted === 'boolean') state.muted = data.muted;
    // 交互增强 b 级：托盘最小化 / 关闭行为开关（app 级 [ui]）。
    state.closeToTray = typeof data.close_to_tray === 'boolean' ? data.close_to_tray : false;
    state.minimizeToTray = typeof data.minimize_to_tray === 'boolean' ? data.minimize_to_tray : false;
    applyPaneLayout();
    applyVolume();
    // 启动自动扫描 / 上次扫描发现的散落 .ncm：提示处理（用户已明确忽略的不重复弹）。
    maybeShowNcmOrganize(data.loose_ncm || [], false);
  }
  updateChrome();
  updateLibraryName();
  // 未配置曲库 → 进入初始化向导；已配置 → 显示主界面并载入库。
  if (!state.library_configured) {
    showInitWizard();
  } else {
    $('init-wizard').hidden = true;
    loadTracks($('search').value);
  }
}

function updateChrome() {
  const st = $('status');
  // 只显示曲库状态；NCM 工具状态由设置页曲源分区承载，左下角不再重复提示。
  st.textContent = state.library_configured ? '曲库就绪' : '未指定曲库';
}

// ---------- 布局风格（横向 / 纵向）----------
// 横向(1) = 曲目详情显示在窗口右侧的独立详情栏；
// 纵向(2) = 详情内联到歌曲列表上方，把 240px 宽度让给列表（长曲名/艺人/专辑不被截断）。
// 每个页面独立配置，切换后立即生效并持久化到配置文件。

// 应用当前页面的布局风格：决定「右侧详情栏」与「内联详情条」的显示与隐藏。
// 实际显隐统一交给 syncDetailVisibility()（公共仲裁，三页一致）。
function applyLayoutStyle() {
  syncDetailVisibility();
  // 布局变化会改变列表可用宽度 → 需要重新按占比分配列宽。
  scheduleColumnWidths();
}

// ---------- 详情渲染（分视图隔离）----------
// 关键修复：详情内容按「页面」隔离存储与渲染。
// 旧实现把同一份曲目详情写进全部 4 个页面的内联详情条（fillInlineDetails），
// 切页时 applyLayoutStyle() 又只切换 hidden、从不重算内容，
// 于是「专辑 / 歌手」页会直接暴露「全部曲库」页选中的歌曲数据。
// 现在每个页面只展示自己的选中内容；未选中时只显示占位提示。

// ---------- 详情区公共组件（DetailRegion） ----------
// 播放列表 / 专辑 / 歌手 三个页面 + 全部曲库页共用同一套渲染与显隐契约，
// 保证四页行为完全一致（需求：控件无数据时直接隐藏，不渲染占位内容）。
//
// 统一契约：
//   1) 数据未选中        → 整个详情区隐藏（不渲染「选中歌曲以查看详情」之类占位）；
//   2) 封面缺失/加载失败 → 封面控件隐藏（不渲染「无封面」字样）；
//   3) 名称为空          → 不渲染名称行；
//   4) 字段值为空        → 该字段行不渲染（不渲染「—」占位）。
//
// 对外只暴露两个方法：renderDetailRegion()（渲染）与 hideDetailRegion()（清空隐藏）。

// 详情区的一组渲染目标：容器 + 封面控件 + 字段控件。
// 三页各自的内联详情条与全局右侧详情栏都用它描述。
function detailTargets(containerId, coverId, fieldsId) {
  return {
    container: $(containerId),
    cover: $(coverId),
    fields: $(fieldsId),
  };
}

// 字段值 → 展示文本。空值（null/undefined/空串）返回 null 表示「该字段不渲染」。
function detailValue(v) {
  if (v === null || v === undefined) return null;
  const s = String(v).trim();
  return s === '' ? null : s;
}

// 构造「名称行 + 字段行」的 HTML 片段。
// name 为空 → 名称行不渲染；value 为空的字段 → 该行不渲染。
function detailRowsHtml(name, fields) {
  let html = '';
  const n = detailValue(name);
  if (n) html += `<div><b>${esc(n)}</b></div>`;
  (fields || []).forEach((f) => {
    const v = detailValue(f.value);
    if (v === null) return; // 该字段无值 → 整行不渲染（不显示「—」）
    html += `<div>${esc(f.label)}：${esc(v)}</div>`;
  });
  return html;
}

// 封面控件：有封面 → <img>；无封面 → 隐藏控件本身。
// 图片加载失败（该文件无内嵌封面）同样隐藏控件，而不是替换成「无封面」文字。
function renderDetailCover(coverEl, localId) {
  if (!coverEl) return;
  if (!localId) {
    coverEl.hidden = true;
    coverEl.innerHTML = '';
    return;
  }
  coverEl.hidden = false;
  coverEl.innerHTML = `<img src="/api/cover/${localId}" alt="" />`;
  const img = coverEl.querySelector('img');
  if (img) {
    img.addEventListener('error', () => {
      coverEl.hidden = true;
      coverEl.innerHTML = '';
    });
  }
}

// 渲染详情区到一组目标（容器 / 封面 / 字段）。
// model = { name, coverId, fields: [{label, value}] }
// 无 model 或渲染不出任何内容 → 整个详情区隐藏（不渲染占位文案）。
function renderDetailRegion(targets, model) {
  const t = targets || {};
  const container = t.container;
  const cover = t.cover;
  const fields = t.fields;
  if (!fields) return false;

  if (!model) {
    hideDetailRegion(targets);
    return false;
  }

  const html = detailRowsHtml(model.name, model.fields);
  if (!html && !model.coverId) {
    // 既无文字也无封面 → 没有任何可展示内容 → 整体隐藏。
    hideDetailRegion(targets);
    return false;
  }

  fields.innerHTML = html;
  renderDetailCover(cover, model.coverId);
  // 字段区无内容时也隐藏，避免出现一个空白块。
  if (fields) fields.hidden = !html;
  // 容器可见性交给调用方（applyLayoutStyle 统一仲裁内联条 / 右侧栏）。
  if (container && container.dataset && container.dataset.detailAutoHidden !== '1') {
    // 内联条由布局仲裁控制显隐，这里不主动改。
  }
  return true;
}

// 清空并隐藏详情区（不留任何占位内容）。
function hideDetailRegion(targets) {
  const t = targets || {};
  if (t.fields) {
    t.fields.innerHTML = '';
    t.fields.hidden = true;
  }
  if (t.cover) {
    t.cover.hidden = true;
    t.cover.innerHTML = '';
  }
}

// 计算某页面当前应展示的详情内容：
//   { type:'track', trx }  → 该页选中了具体歌曲
//   { type:'group', ... }  → 该页选中了分组（专辑 / 歌手 / 播放列表）但未选具体歌曲
//   null                   → 该页没有任何选中
function viewDetail(view) {
  const sel = state.viewSelection[view];
  if (sel && sel.trx) return { type: 'track', trx: sel.trx };
  if (view === 'albums' && state.currentAlbum) {
    return {
      type: 'group',
      kind: '专辑',
      title: state.currentAlbum.name,
      count: state.currentAlbum.track_count,
      coverId: (state.albumTracks[0] || {}).local_id || null,
    };
  }
  if (view === 'artists' && state.currentArtist) {
    return {
      type: 'group',
      kind: '歌手',
      title: state.currentArtist.name,
      count: state.currentArtist.track_count,
      coverId: (state.artistTracks[0] || {}).local_id || null,
    };
  }
  if (view === 'playlists' && state.currentPlaylist) {
    return {
      type: 'group',
      kind: '播放列表',
      title: state.currentPlaylist.name,
      count: state.currentPlaylist.track_count,
      coverId: (state.playlistTracks[0] || {}).local_id || null,
    };
  }
  // v0.12.7：曲源页选中分组但未选具体歌曲时，右侧详情栏显示曲源名与歌曲数。
  if (view === 'sources' && state.currentSource) {
    return {
      type: 'group',
      kind: '曲源',
      title: state.currentSource.name,
      count: state.currentSource.track_count,
      coverId: (state.sourceTracks[0] || {}).local_id || null,
    };
  }
  return null;
}

// 把 viewDetail() 的结果转成公共组件使用的统一 model。
// 曲目详情 → 字段集合；分组详情 → 名称 + 类型 + 曲目数。
function detailModelOf(view) {
  const d = viewDetail(view);
  if (!d) return null;
  if (d.type === 'track') {
    const t = d.trx || {};
    return {
      name: t.name,
      coverId: t.local_id || null,
      fields: [
        { label: '艺人', value: t.artist },
        { label: '专辑', value: t.album },
        { label: '格式', value: t.format },
        { label: '采样率', value: t.sample_rate ? t.sample_rate + ' Hz' : null },
        { label: '位深', value: t.bit_depth ? t.bit_depth + ' bit' : null },
        { label: '码率', value: t.bitrate ? Math.round(t.bitrate / 1000) + ' kbps' : null },
        { label: '声道', value: t.channels || null },
        { label: '时长', value: t.duration_ms ? fmtTime(t.duration_ms) : null },
        { label: '质量层级', value: t.quality_tier },
        // #28：流派 / 年份。genre 空串或 year 为空时 detailValue → null，
        // detailRowsHtml 会整行跳过，遵守「空字段不渲染、不用 —」约定。
        { label: '流派', value: t.genre || null },
        { label: '年份', value: t.year || null },
        // 空值传 null → detailRowsHtml 会整行跳过，不渲染「—」占位。
        { label: '导入时间', value: fmtDateTimeFull(t.imported_at) || null },
        { label: '创建时间', value: fmtDateTimeFull(t.created_at) || null },
      ],
    };
  }
  // 分组（专辑 / 歌手 / 播放列表）：名称 + 类型 + 曲目数。
  return {
    name: d.title,
    coverId: d.coverId || null,
    fields: [
      { label: '类型', value: d.kind },
      { label: '曲目数', value: d.count },
    ],
  };
}

// 把某页面的详情写入该页的「内联详情条」（纵向布局时显示在列表上方）。
// 渲染完成后立即重算显隐：内容为空 → 该页内联条隐藏（不留占位）。
function renderViewDetail(view) {
  // v0.12.7：曲源页没有独立内联详情条，只走共用右侧详情栏；
  // 不可回退写 lib-detail，否则切回曲库会残留曲源选中态。
  if (view === 'sources') { syncDetailVisibility(); return; }
  // #32：收藏复用曲库的内联详情条（无独立 DOM），其余页面各自一份。
  const c = LAYOUT_VIEWS[view] || LAYOUT_VIEWS.library;
  if (!c) return;
  const targets = detailTargets(c.wrap, c.cover, c.fields);
  const ok = renderDetailRegion(targets, detailModelOf(view));
  state.detailShown[view] = ok;
  syncDetailVisibility();
}

// 把「当前页面」的详情写入全局右侧独立详情栏（横向布局时可见）。
// 右侧栏是四个页面共用的单一 DOM，必须在切页时按新页面重算，否则会残留上一页内容。
function renderActiveDetail() {
  const view = state.activeTab;
  const targets = detailTargets(null, 'detail-cover', 'detail-fields');
  const ok = renderDetailRegion(targets, detailModelOf(view));
  state.detailShown[view] = ok;
  syncDetailVisibility();
}

// 详情区的显隐仲裁（三页一致的唯一入口）：
//   · 横向（1）：显示全局右侧详情栏；隐藏所有内联条。
//   · 纵向（2）：隐藏右侧详情栏；显示**当前页**的内联条。
//   · 无论哪种布局，若该页没有任何可展示内容 → 对应详情区整体隐藏，
//     不渲染「无封面」「暂无信息」等占位内容。
function syncDetailVisibility() {
  const view = state.activeTab;
  // #32：收藏没有自己的内联详情条 / 布局配置，复用曲库的那一份；
  // 详情内容仍取 state.detailShown[view]（收藏自己的选中态），保持分视图隔离。
  // v0.12.7：曲源页无独立布局配置，复用曲库（横向右侧详情栏）。
  const styleView = (view === 'favorites' || view === 'sources') ? 'library' : view;
  const inline = (state.layoutStyles[styleView] || 1) === 2;
  // FR-25：纵向布局时右侧详情栏隐藏，列表内图标按钮需贴列表右缘。
  document.body.classList.toggle('detail-inline', inline);
  // 右侧独立详情栏：纵向时隐藏（宽度让给列表）。
  // 横向且已收起时窄条必须常驻（图标按钮就在这里，是唯一的展开入口），
  // 只有「展开态 + 该页无任何内容」才整体隐藏，避免留下一条空栏。
  $('detail-pane').hidden = inline || (!state.detailShown[view] && !paneCollapsed('detail'));
  // 内联详情条：仅当前页 + 纵向 +（有内容 或 处于收起态）时显示。
  VIEW_KEYS.forEach((v) => {
    const el = $(LAYOUT_VIEWS[v].wrap);
    if (el) el.hidden = !(inline && v === styleView && (state.detailShown[view] || paneCollapsed('detail')));
  });
  // FR-25 第五轮：详情细条始终跟着「当前真正可见的详情区」走（横向=右侧栏，纵向=内联水平条）。
  const dh = $('handle-detail');
  if (dh) {
    const host = inline ? $(LAYOUT_VIEWS[styleView].wrap) : $('detail-pane');
    if (host && dh.parentNode !== host) host.appendChild(dh);
  }
  // 布局方向变了 → 详情栏按钮的箭头方向跟着切（横向左右 / 纵向上下）。
  updatePaneHandleArrows();
}

// 同步表格高亮：清除所有表格的选中态，再把当前页的选中项标回去。
function syncSelectionHighlight(view) {
  document.querySelectorAll('.track-table tbody tr.selected')
    .forEach((r) => r.classList.remove('selected'));
  const sel = state.viewSelection[view];
  if (!sel || !sel.track) return;
  const box = $(VIEW_BOX[view]);
  if (!box) return;
  const tr = box.querySelector('tr[data-id="' + sel.track.local_id + '"]');
  if (tr) tr.classList.add('selected');
}

// 顶栏曲库名称：优先显示用户自定义名称；已配置但未命名时回退为「未命名曲库」。
function updateLibraryName() {
  const el = $('library-name');
  if (!el) return;
  if (state.library_name) {
    el.textContent = state.library_name;
    el.title = '曲库名称：' + state.library_name + '（点击 ✎ 切换曲库）';
  } else if (state.library_configured) {
    el.textContent = '未命名曲库';
    el.title = '未命名曲库（点击 ✎ 切换曲库）';
  } else {
    el.textContent = '';
    el.title = '';
  }
}

// ---------- 关于（应用元信息） ----------
// 数据源：后端 /api/about（编译期常量 mt_core::app），与原生窗口标题「K music vX.Y.Z」同源。
async function loadAbout() {
  const { data } = await api('/api/about');
  if (data) state.about = data;
  renderAbout();
}

// 渲染版本号（文档标题）与「关于」面板内容。
function renderAbout() {
  const a = state.about;
  // 版本号：文档标题（窗口标题栏已显示版本号，顶栏不再重复徽标）。
  const ver = a && a.version ? a.version : '';
  if (ver) {
    document.title = (a.name || 'K music') + ' v' + ver;
  }
  if (!a) return;
  const set = (id, text) => {
    const el = $(id);
    if (el) el.textContent = text;
  };
  set('about-name', a.name || '—');
  set('about-version', a.version ? 'v' + a.version : '—');
  set('about-author', a.author || '—');
  set('about-license', a.license || '—');
  set('about-tech', a.tech || '—');
  set('about-desc', a.description || '—');
  const home = $('about-homepage');
  if (home) {
    if (a.homepage) {
      home.textContent = a.homepage;
      home.href = a.homepage;
    } else {
      home.textContent = '—';
      home.removeAttribute('href');
    }
  }
}

// ---------- 配置文件版本状态 ----------
// 数据源：后端 /api/status 的 config_* 字段（见 core/src/config.rs 的 ConfigStatus）。
const COMPAT_LABELS = {
  compatible: '兼容（已保留既有配置）',
  legacy: '旧格式（内容已保留并补写版本信息）',
  conflict: '冲突（已备份原文件并重建）',
};

// 渲染「配置文件」分区：版本信息 + 兼容判定 + 冲突原因。
function renderConfigStatus() {
  const set = (id, text) => {
    const el = $(id);
    if (el) el.textContent = text;
  };
  const appOk = !state.config_compat || state.config_compat === 'compatible';
  const libOk = !state.library_config_compat || state.library_config_compat === 'compatible';
  const normal = appOk && libOk;
  const summary = $('cfg-summary');
  if (summary) {
    summary.textContent = normal
      ? '配置文件状态：正常（app 与曲库配置均兼容）'
      : '配置文件状态：存在兼容性提示，详见下方版本信息';
  }
  const detail = $('cfg-detail');
  if (detail) detail.hidden = normal;
  if (!normal) {
    set('cfg-version', state.config_version ? 'v' + state.config_version : '—');
    set('cfg-min-version', state.config_min_version ? 'v' + state.config_min_version : '—');
    set('cfg-app-version', state.app_version ? 'v' + state.app_version : '—');
    set('cfg-library-version', state.library_config_version ? 'v' + state.library_config_version : '—');
    const label = COMPAT_LABELS[state.config_compat] || state.config_compat || '—';
    const libLabel = COMPAT_LABELS[state.library_config_compat];
    set('cfg-compat', libLabel ? label + ' / 曲库：' + libLabel : label);
  }
  // 冲突原因：明确展示（app 级与曲库级都列出）。
  const reasons = [];
  if (state.config_conflict) reasons.push('app 配置：' + state.config_conflict);
  if (state.library_config_conflict) reasons.push('曲库配置：' + state.library_config_conflict);
  const box = $('cfg-conflict');
  if (box) {
    if (reasons.length) {
      box.textContent = '⚠ 配置冲突：' + reasons.join('；');
      box.hidden = false;
    } else {
      box.textContent = '';
      box.hidden = true;
    }
  }
}

// ---------- 曲库列表 ----------
// v0.16.2：并发序号守卫（与 plReq / albumReq / artistReq 同构）+ 四态状态条。
// 防抖只挂在搜索框 input 事件上，不进 loadTracks 本体（筛选 / 切 tab / 重组后刷新即时生效）。
let tracksReq = 0;
let searchDebounce = null;

function setTrackListStatus(st, err) {
  state.trackListState = st;
  state.trackListError = err || '';
  const box = $('track-load-status');
  if (!box) return;
  // 只在「全部曲库」视图展示；收藏视图复用同一表格，不归曲库加载状态管。
  if (state.activeTab !== 'library') { box.hidden = true; return; }
  if (st === 'ready') { box.hidden = true; box.textContent = ''; return; }
  box.hidden = false;
  if (st === 'loading') {
    box.innerHTML = '<span>正在加载曲库…</span>';
  } else if (st === 'empty') {
    const filtered = !!(state.tracksQuery || state.trackFilters.quality || state.trackFilters.unavailable);
    box.innerHTML = '<span>' + (filtered ? '没有匹配歌曲' : '曲库暂无歌曲') + '</span>';
  } else if (st === 'error') {
    box.innerHTML = '<span>' + esc(state.trackListError) + '</span>' +
      '<button type="button" class="md-btn sm" id="track-load-retry">重试</button>';
  }
}

async function loadTracks(q) {
  state.tracksQuery = q || '';
  const seq = ++tracksReq;
  const params = new URLSearchParams();
  if (state.tracksQuery) params.set('q', state.tracksQuery);
  if (state.trackFilters.quality) params.set('quality', state.trackFilters.quality);
  if (state.trackFilters.unavailable) params.set('unavailable', 'true');
  setTrackListStatus('loading');
  const { ok, data } = await api('/api/tracks?' + params.toString());
  if (seq !== tracksReq) return; // 旧响应直接丢弃：不改数据、不渲染
  if (!ok || !data || !Array.isArray(data.tracks)) {
    // 失败保留旧列表，不清空 state.tracks / 不 renderTracks；仅状态条提示错误 + 重试。
    const why = (data && data.error) || '曲库加载失败';
    setTrackListStatus('error', why + '，已保留上次结果');
    return;
  }
  state.tracks = data.tracks;
  setTrackListStatus(data.tracks.length ? 'ready' : 'empty');
  renderTracks();
}

// 切换某曲目收藏标记（按 local_id）。更新本地数据 + 所有表格里对应的心形按钮。
async function toggleFavorite(id) {
  const list =
    state.tracks.find((t) => t.local_id === id) ||
    state.playlistTracks.find((t) => t.local_id === id) ||
    state.albumTracks.find((t) => t.local_id === id) ||
    state.artistTracks.find((t) => t.local_id === id);
  const next = !(list && list.favorite);
  try {
    const { data } = await api('/api/tracks/' + id + '/favorite', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ favorite: next }),
    });
    if (!data || !data.ok) {
      setHint((data && data.error) || '收藏失败');
      return;
    }
    if (list) list.favorite = next;
    // #31：增量维护「喜爱」缓存，不重新请求 /api/favorites；
    // 收藏模式下取消收藏立即从列表移除（实时推导语义）。
    if (next) {
      if (!state.favoriteTracks.some((t) => Number(t.local_id) === Number(id))) {
        state.favoriteTracks.push(Object.assign({}, list || { local_id: id }, { favorite: true }));
      }
    } else {
      state.favoriteTracks = state.favoriteTracks.filter((t) => Number(t.local_id) !== Number(id));
    }
    state.favoriteCount = state.favoriteTracks.length;
    syncFavoriteCount();
    if (state.activeTab === 'favorites') renderTracks();
    document.querySelectorAll('.fav-btn[data-id="' + id + '"]').forEach((b) => {
      b.textContent = next ? '♥' : '♡';
      b.classList.toggle('active', next);
      b.setAttribute('aria-pressed', String(next));
    });
  } catch (e) {
    setHint('收藏请求失败：' + (e && e.message ? e.message : e));
  }
}

// 收藏心形点击：捕获阶段委托，stopPropagation 避免误触发行选中 / 详情。
document.addEventListener(
  'click',
  (e) => {
    const btn = e.target.closest('.fav-btn');
    if (!btn) return;
    e.stopPropagation();
    e.preventDefault();
    toggleFavorite(Number(btn.dataset.id));
  },
  true
);

// #33：全部曲库筛选条 = 四档音质平铺按钮（互斥单选），不再有下拉与「已收藏」入口。
// 仍只影响本视图；收藏集合由第 5 个一级视图 /api/favorites 独立推导。
function setQualityFilter(value) {
  state.trackFilters.quality = String(value || '');
  const seg = $('filter-quality');
  if (seg) {
    seg.querySelectorAll('.filter-chip').forEach((b) => {
      const on = (b.dataset.quality || '') === state.trackFilters.quality;
      b.classList.toggle('active', on);
      b.setAttribute('aria-pressed', String(on));
    });
  }
  loadTracks(state.tracksQuery);
}
// v0.17.7：仅看不可访问（正向筛选，仅全部曲库视图生效）。
function setUnavailableFilter(on) {
  state.trackFilters.unavailable = !!on;
  const seg = $('filter-unavailable');
  if (seg) {
    seg.querySelectorAll('.filter-chip').forEach((b) => {
      const active = state.trackFilters.unavailable;
      b.classList.toggle('active', active);
      b.setAttribute('aria-pressed', String(active));
    });
  }
  loadTracks(state.tracksQuery);
}
(function initTrackFilters() {
  const seg = $('filter-quality');
  if (seg) {
    seg.addEventListener('click', (e) => {
      const b = e.target.closest('.filter-chip');
      if (b && seg.contains(b)) setQualityFilter(b.dataset.quality);
    });
  }
  const ua = $('filter-unavailable');
  if (ua) {
    ua.addEventListener('click', (e) => {
      const b = e.target.closest('.filter-chip');
      if (b && ua.contains(b)) setUnavailableFilter(b.dataset.unavailable === 'true' && !state.trackFilters.unavailable);
    });
  }
})();

// ---------- #31/#32：「喜爱」独立智能列表（第 5 个一级视图）----------
// 实时从 /api/favorites 推导收藏集合（绝不写 playlists 表）。boot / 切到本视图时取权威值，
// 之后由 toggleFavorite() 增量维护；播放队列是 local_id[] 快照、生成后不读 DOM。
async function loadFavorites(render) {
  const { ok, data } = await api('/api/favorites');
  // 后端暂不可用时保留上一次缓存，避免侧栏计数 / 整组播放被误清零；
  // 但仍按当前缓存重绘，绝不把「全部曲库」的行残留在收藏视图里。
  if (!ok || !data || !Array.isArray(data.tracks)) {
    syncFavoriteCount();
    if (render) renderTracks();
    return;
  }
  state.favoriteTracks = data.tracks;
  state.favoriteCount = state.favoriteTracks.length;
  syncFavoriteCount();
  if (render) renderTracks();
}

function syncFavoriteCount() {
  const c = $('fav-count');
  if (c) c.textContent = String(state.favoriteCount);
  const c2 = $('favorites-count');
  if (c2) c2.textContent = `共 ${state.favoriteCount} 首`;
}

// 整组播放：双击侧栏入口或点「播放全部」；走既有 startPlayback(fullGroup)，
// 队列来源 = sourceTracks('favorites')，buildQueue() 算法一行不改。
async function playAllFavorites() {
  if (state.activeTab !== 'favorites') switchTab('favorites');
  // 先取权威集合：boot 缓存可能尚未回来或已过期，直接用它生成 fullGroup 会漏播。
  await loadFavorites(false);
  if (!state.favoriteTracks.length) { setHint('还没有收藏的歌曲'); return; }
  startPlayback(state.favoriteTracks[0], 'favorites', { fullGroup: true });
}

(function initFavoritesEntry() {
  const play = $('favorites-play');
  if (play) play.addEventListener('click', playAllFavorites);
  // #35：无「返回曲库」按钮；喜爱与专辑/歌手同级，返回走侧栏「全部曲库」Tab。
  // boot 缓存一次集合供侧栏计数与双击整组播放；不渲染，首屏曲库表格零行为变化。
  loadFavorites(false);
})();

// ---------- #32：无边框窗口自绘控制条 ----------
// 隐藏系统标题栏后，拖动与三个按钮都走本机 REST，由 GUI 壳调用 Tauri 窗口 API。
// 不依赖 WebView IPC 权限；keepalive 保证拖拽请求在快速交互中不被丢弃。
function windowControl(action) {
  fetch('/api/window/control', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action }),
    keepalive: true,
  }).catch(() => {});
}

// v0.14.13：最大化按钮图标必须跟随窗口真实状态（□=最大化 / ❐=还原）。
// 后端在 toggle / 启动恢复 / 外部 Resized 后写入权威值，前端首屏与 resize 时
// 从 /api/status 回读；点击时即时翻转以保证零延迟（后端动作是确定性 toggle），
// 避免 Tauri maximize() 异步状态导致读回滞后一帧。
let windowMaximized = false;
function applyWindowMaximizeState(on) {
  const btn = $('win-maximize');
  if (!btn) return;
  windowMaximized = !!on;
  const glyph = windowMaximized ? '❐' : '□';
  const label = windowMaximized ? '还原' : '最大化';
  if (btn.textContent !== glyph) btn.textContent = glyph;
  if (btn.title !== label) btn.title = label;
  if (btn.getAttribute('aria-label') !== label) btn.setAttribute('aria-label', label);
}

// 外部方式（双击标题栏 / Win+↑ / 贴靠 / 任务栏）最大化不走本按钮点击，
// 靠 resize 事件回读权威态；150ms 防抖 + 在途去重，避免拖动窗口时刷请求。
let maxResizeTimer = null;
let maxSyncInFlight = false;
async function syncWindowMaximizeFromStatus() {
  if (maxSyncInFlight) return;
  maxSyncInFlight = true;
  try {
    const { data } = await api('/api/status');
    if (data && typeof data.maximized === 'boolean') applyWindowMaximizeState(data.maximized);
  } catch (_) {
    // 后端尚未就绪 / CLI serve 模式无窗口：保留当前图标，不打扰用户。
  } finally {
    maxSyncInFlight = false;
  }
}
window.addEventListener('resize', () => {
  clearTimeout(maxResizeTimer);
  maxResizeTimer = setTimeout(syncWindowMaximizeFromStatus, 150);
});

(function initWindowControls() {
  const map = {
    'win-minimize': 'minimize',
    'win-maximize': 'toggle-maximize',
    'win-close': 'close',
  };
  Object.keys(map).forEach((id) => {
    const el = $(id);
    if (el) el.addEventListener('click', () => {
      if (id === 'win-maximize') applyWindowMaximizeState(!windowMaximized);
      windowControl(map[id]);
    });
  });
  // v0.12.10：主界面背景空白区也能拖动窗口。判定用 matches 而非 closest —— 只有事件
  // **直接命中**背景容器才算，否则点内容 / 控件时会因其祖先是背景容器而误拖窗口。
  // 播放栏（.player 及其子级）按需求不纳入；两套皮肤共用同一份选择器。
  const BG_DRAG = 'html, body, .layout, .sidebar, .list-pane, .detail-pane, .playlists-pane, .pl-sidebar, .pl-main, .pl-content, .list-scroll, .detail-scroll, .filter-bar, .favorites-bar, .pl-sidebar-head, .pl-main-head, .detail-pane-head';
  const BG_BLOCK = 'button, input, select, textarea, a, label, [contenteditable], .win-controls, .topbar-search, .topbar-extras, #library-name, tr, td, th, li, img, svg, canvas, video, audio, .track-table, .pl-list, .tab-btn, .pane-resizer, .pane-handle, .col-resizer, .modal-mask, .modal, .now-view, .lyrics-drawer, .queue-drawer, .progress-panel, .progress-pill, .update-bar, .hint-bar, .player, .brand, .status, .actions, .cover, .detail-fields, .lyric-line, .gcard, .group-row, .ctx-menu';
  // v0.12.13：原生滚动条（::-webkit-scrollbar）是伪元素，拖它时 mousedown 的
  // e.target 就是滚动容器本身；若该容器也在 BG_DRAG 里，会把「拖滚动条」误判成
  // 「拖背景」并发 drag，start_dragging() 抢走鼠标后滚动条就拖不动了。
  // 按几何位置把垂直 / 水平滚动条带排除：带宽 = offset - client（含边框），
  // 只有确实可滚动且指针落在右 / 下带内才算命中；事件目标链与 elementFromPoint
  // 命中链都查，兼容 WebView2 把目标给到祖先或子元素的情况。
  const dragOnScrollbar = (x, y, target) => {
    const seen = new Set();
    const roots = [target, document.elementFromPoint(x, y)];
    for (const root of roots) {
      let node = root instanceof Element ? root : null;
      while (node && !seen.has(node)) {
        seen.add(node);
        const v = node.offsetWidth - node.clientWidth;
        const h = node.offsetHeight - node.clientHeight;
        if (v > 0 || h > 0) {
          const r = node.getBoundingClientRect();
          if ((v > 0 && node.scrollHeight > node.clientHeight + 1 && x >= r.right - v) ||
              (h > 0 && node.scrollWidth > node.clientWidth + 1 && y >= r.bottom - h)) return true;
        }
        node = node.parentElement;
      }
    }
    return false;
  };
  // 顶栏空白区（#32 原逻辑，含交互元素排除）与背景空白区共用一条委托；
  // 双击的第二下不重复发拖动请求。
  // v0.16.14：设置弹窗打开时遮罩盖住整窗，标题栏空白区作为等价拖动把手（关闭按钮除外）。
  document.addEventListener('mousedown', (e) => {
    if (e.button !== 0 || e.detail > 1) return;
    if (e.target.closest('#settings-modal .settings-head')) {
      if (e.target.closest('button, input, select, textarea, a, label, [contenteditable]')) return;
      windowControl('drag');
      return;
    }
    if (e.target.closest('.topbar, .toolbar')) {
      if (e.target.closest('button, input, select, textarea, a, label, [contenteditable], .win-controls, #library-name, .topbar-search, .topbar-extras')) return;
      windowControl('drag');
      return;
    }
    if (!(e.target instanceof Element) || !e.target.matches(BG_DRAG)) return;
    if (e.target.closest(BG_BLOCK)) return;
    if (dragOnScrollbar(e.clientX, e.clientY, e.target)) return;
    windowControl('drag');
  });
})();

function renderTracks() {
  const head = $('track-head');
  const body = $('track-body');
  // 状态条只与曲库视图绑定：收藏视图复用同一表格，此时状态条必须隐藏。
  const statusBox = $('track-load-status');
  if (statusBox && state.activeTab !== 'library') statusBox.hidden = true;
  // 表头：最左侧固定 # 序号列 + 可见业务列（按 state.columns 顺序）。
  // 业务列带 data-col-key，供列宽拖拽与占比分配使用。
  const cols = visibleTrackCols();
  head.innerHTML = '<th class="idx-col">#</th>' + trackHeadHtml(cols);
  body.innerHTML = '';
  libraryTracks().forEach((t, i) => {
    const tr = document.createElement('tr');
    tr.dataset.id = t.local_id;
    tr.innerHTML = idxCellHtml(t, i) + trackCellsHtml(t, cols);
    tr.addEventListener('click', () => selectTrack(t, tr));
    tr.addEventListener('dblclick', () => { selectTrack(t, tr); playTrackToggleIfCurrent(t); });
    tr.addEventListener('contextmenu', (e) => openTrackContextMenu(e, t, 'library', tr));
    body.appendChild(tr);
  });
  setupColumnResize($('track-table'));
  updateTrackCount();
  autoSelectFirst();
}

// ---------- 歌曲列表列宽可调 ----------
// 列宽以「占比 %」存储（state.columnWidths，八列之和恒为 100，持久化在配置文件 [ui] 段）。
// 渲染时把占比换算成像素宽应用到表头 th；拖拽表头分隔线实时调整并回写占比。
// 因 table-layout:fixed，占比之和恒定 → 拖拽不会撑破表格，窗口缩放时按占比重新分配。

// 表格中可见业务列的归一化占比（%）：只在该表格实际显示的列之间归一化。
function normalizedColPcts(keys) {
  const vals = keys.map((k) => Number(state.columnWidths[k]) || 0);
  const sum = vals.reduce((a, b) => a + b, 0);
  if (sum <= 0) return keys.map(() => 100 / keys.length);
  return vals.map((v) => (v / sum) * 100);
}

// 计算表格里「业务列可用总宽」：滚动容器宽 - 固定列（序号 / 播放 / 移除）宽度。
// ⚠️ 基准必须用**容器**宽度，不能用表格自身宽度：table-layout:fixed 下浏览器在尚未
// 显式设列宽时会按表头内容把表格撑宽（8 列实测撑到 1216px，而容器只有 904px），
// 若拿这个已撑宽的值再分配列宽就形成正反馈，末尾几列会被永远挤出可视区。
// 向下取整，保证「业务列宽 + 固定列宽 ≤ 容器宽」。
function tableAvailWidth(table) {
  const host = table.parentElement;
  const total = host
    ? host.getBoundingClientRect().width
    : table.getBoundingClientRect().width;
  if (!total) return 0;
  let fixed = 0;
  table.querySelectorAll('thead th').forEach((th) => {
    if (!th.dataset.colKey) fixed += th.getBoundingClientRect().width;
  });
  return Math.max(Math.floor(total - fixed), 60);
}

// 把占比分配到表头 th 的像素宽度上。
// 用「累计取整」而非逐列取整，保证各列宽度之和**恰好**等于 avail（否则逐列舍入误差
// 累积会让总宽超出 avail 几个像素，导致表格被撑宽）。
function applyWidthsToThs(cols, pcts, avail) {
  const n = cols.length;
  let acc = 0;
  let prev = 0;
  for (let i = 0; i < n; i++) {
    acc += pcts[i];
    const end = i === n - 1 ? avail : Math.round((avail * acc) / 100);
    cols[i].style.width = Math.max(end - prev, 1) + 'px';
    prev = end;
  }
}

// 按当前占比重新分配所有歌曲表格的列宽（窗口缩放 / 布局切换 / 数据刷新后调用）。
function applyColumnWidths(table) {
  if (!table) return;
  const cols = Array.from(table.querySelectorAll('thead th[data-col-key]'));
  if (!cols.length) return;
  const avail = tableAvailWidth(table);
  if (!avail) return; // 面板隐藏（宽度为 0）→ 跳过，切回时再算
  const keys = cols.map((th) => th.dataset.colKey);
  applyWidthsToThs(cols, normalizedColPcts(keys), avail);
}

// 汇总调度：一次刷新所有可见表格的列宽（合并同一帧内的多次请求）。
let colWidthRaf = 0;
// 拖拽进行中标记：拖拽期间**不能**重算列宽，否则会覆盖用户正在拖出来的宽度。
let colDragActive = false;
function scheduleColumnWidths() {
  if (colWidthRaf) cancelAnimationFrame(colWidthRaf);
  colWidthRaf = requestAnimationFrame(() => {
    colWidthRaf = 0;
    // 拖拽中直接跳过（拖拽结束会自行写回比例并落盘）。
    if (colDragActive) return;
    document.querySelectorAll('.track-table').forEach((t) => applyColumnWidths(t));
  });
}
window.addEventListener('resize', scheduleColumnWidths);

// 为业务列表头挂上拖拽手柄，并应用当前列宽。
function setupColumnResize(table) {
  if (!table) return;
  const cols = Array.from(table.querySelectorAll('thead th[data-col-key]'));
  cols.forEach((th, i) => {
    // 最后一列没有右邻列可交换宽度 → 不加手柄。
    if (i === cols.length - 1) return;
    if (th.querySelector('.col-resizer')) return;
    th.classList.add('col-resizable');
    const handle = document.createElement('span');
    handle.className = 'col-resizer';
    handle.title = '拖动调整列宽';
    th.appendChild(handle);
    handle.addEventListener('mousedown', (e) => startColResize(e, table, cols, i));
  });
  applyColumnWidths(table);
}

// 开始拖拽列分隔线：该列变宽、右邻列等量变窄（总宽不变）。
// 拖拽过程按**像素**计算（与用户直觉一致：分隔线移动了多少像素），
// 松手时再换算成占比写回配置。
function startColResize(e, table, cols, idx) {
  const next = cols[idx + 1];
  if (!next) return;
  e.preventDefault();
  e.stopPropagation();
  const keys = cols.map((th) => th.dataset.colKey);
  const avail = tableAvailWidth(table);
  if (!avail) return;
  const startA = cols[idx].getBoundingClientRect().width;
  const startB = next.getBoundingClientRect().width;
  const totalPx = Math.round(startA + startB); // 两列宽度之和保持不变
  // 自适应像素下限：显示「导入时间」时可见列变多，窄列（音质/时长/文件大小）会被
  // 归一化压到 48px 以下，此时若两列都硬钳到 MIN_COL_PX=48，两列总额 < 96px 会让
  // 下面两条 clamp 互相覆盖、无论往哪拖都被钉死成固定值。下限取「不超过列对总宽
  // 一半再留 12px 余量」，保证这对列有可拖动空间；24px 是单元格左右内边距之和
  // （border-box 下不可再压缩），低于它浏览器会强制撑宽、把表格顶破，故以此为硬底。
  // 列对足够宽时下限仍等于 48px，原行为不变（见 demo B 组「艺人列 ≥48px」断言）。
  const pairMin = Math.min(MIN_COL_PX, Math.max(24, Math.floor(totalPx / 2) - 12));
  const startX = e.clientX;
  const handle = e.currentTarget;
  handle.classList.add('active');
  document.body.classList.add('col-resizing');
  colDragActive = true;

  const onMove = (ev) => {
    // 表格若在拖拽期间被重新渲染（表头元素已被替换），拖拽失效 → 直接放弃。
    if (!cols[idx].isConnected) return;
    let aPx = Math.round(startA + (ev.clientX - startX));
    let bPx = totalPx - aPx;
    // 任一列不得窄于自适应下限（受单元格内边距限制）；差额回推给对方。
    if (bPx < pairMin) { bPx = pairMin; aPx = totalPx - bPx; }
    if (aPx < pairMin) { aPx = pairMin; bPx = totalPx - aPx; }
    cols[idx].style.width = aPx + 'px';
    next.style.width = bPx + 'px';
  };
  const onUp = () => {
    document.removeEventListener('mousemove', onMove);
    document.removeEventListener('mouseup', onUp);
    handle.classList.remove('active');
    document.body.classList.remove('col-resizing');
    colDragActive = false;
    // 表头元素已被替换（表格重新渲染）→ 本次拖拽结果作废，不写回。
    if (!cols[idx].isConnected) return;
    // 像素 → 占比：**只**把本次拖拽的像素变化折算回被拖动的两列，其余列保持原值。
    // 不能按「各列实际渲染宽度 / 总渲染宽度」整体重算：窄窗口下浏览器会按单元格
    // 内边距给窄列施加最小宽度（MIN_COL_PX），渲染占比与配置占比并不一致，
    // 整体重算会把这些纯布局误差永久写进配置，甚至让没被拖动的列比例漂移。
    const finalA = cols[idx].getBoundingClientRect().width;
    const deltaPx = finalA - startA;
    const keyA = keys[idx], keyB = keys[idx + 1];
    const storedA = Number(state.columnWidths[keyA]) || 0;
    const storedB = Number(state.columnWidths[keyB]) || 0;
    const pair = storedA + storedB;
    const deltaPct = (deltaPx / (totalPx || 1)) * pair;
    // 两列之和保持 pair 不变 → 隐藏列份额不变 → 八列存储占比之和仍为 100。
    const nextA = Math.min(Math.max(storedA + deltaPct, MIN_COL_PCT), pair - MIN_COL_PCT);
    const nextB = pair - nextA;
    state.columnWidths[keyA] = Math.round(nextA * 100) / 100;
    state.columnWidths[keyB] = Math.round(nextB * 100) / 100;
    saveColumnWidths();
    renderColWidthUI();
  };
  document.addEventListener('mousemove', onMove);
  document.addEventListener('mouseup', onUp);
}

// 持久化列宽比例（只提交 column_widths，后端就地更新配置文件的 [ui] 段）。
async function saveColumnWidths() {
  // 先按后端同一套规则归一化，保证内存状态与落盘内容一致。
  normalizeStoredWidths();
  const { data } = await api('/api/ui-prefs', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ column_widths: state.columnWidths }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '保存列宽比例失败'); return; }
}

// 把八列占比归一化：单列低于下限 → 恢复默认；总和 ≠ 100 → 等比缩放到 100。
// 与后端 config::UiConfig::sanitize 规则一致，避免前后端比例不一致。
function normalizeStoredWidths() {
  ALL_COLUMN_KEYS.forEach((k) => {
    const v = Number(state.columnWidths[k]) || 0;
    if (!isFinite(v) || v < MIN_COL_PCT) state.columnWidths[k] = DEFAULT_COL_WIDTHS[k];
  });
  const sum = ALL_COLUMN_KEYS.reduce((a, k) => a + (Number(state.columnWidths[k]) || 0), 0);
  if (sum > 0 && Math.abs(sum - 100) > 0.01) {
    const scale = 100 / sum;
    ALL_COLUMN_KEYS.forEach((k) => {
      state.columnWidths[k] = Math.round((Number(state.columnWidths[k]) || 0) * scale * 100) / 100;
    });
  }
}

// 渲染后默认选中：保持当前选中的曲目（若仍在过滤后列表内），否则选中第一行。
// 保证打开主界面时右侧详情面板默认展示第一首歌曲，列表高亮也为首行。
function autoSelectFirst() {
  const view = tracksView();
  const list = libraryTracks();
  if (!list.length) {
    state.current = null;
    state.viewSelection[view] = null;
    clearDetail();
    return;
  }
  // 只认当前表格所属视图自己的选中态，避免其它页面的选中串进来。
  const prev = state.viewSelection[view];
  let sel = null;
  if (prev && prev.track) {
    sel = list.find((t) => t.local_id === prev.track.local_id) || null;
  }
  if (!sel) sel = list[0];
  const tr = $('track-body').querySelector('tr[data-id="' + sel.local_id + '"]');
  if (!tr) return;
  // 仅当切换了选中对象时才重新拉取详情，避免搜索过滤时反复请求。
  if (!prev || !prev.track || prev.track.local_id !== sel.local_id) {
    selectTrack(sel, tr);
  } else {
    tr.classList.add('selected');
  }
}

// 当前表格无曲目 / 未选中时清空该视图的详情（并同步右侧详情栏）。
function clearDetail() {
  const view = tracksView();
  state.viewSelection[view] = null;
  renderViewDetail(view);
  if (state.activeTab === view) renderActiveDetail();
}

// 刷新按钮旁的统计：当前列表（已按搜索过滤）的歌曲数量。
function updateTrackCount() {
  $('track-count').textContent = `共 ${libraryTracks().length} 首`;
}

// 详情请求的并发令牌：连续快速点击时只认最后一次请求的结果，
// 避免「先点的后返回」把新选中的歌曲覆盖成旧数据。
let detailSeq = 0;

async function selectTrack(t, tr) {
  // 归属页面：点击发生在哪个页面就写入哪个页面的选中态，四个页面互不干扰。
  const view = state.activeTab;
  document.querySelectorAll('.track-table tbody tr.selected')
    .forEach((r) => r.classList.remove('selected'));
  if (tr) tr.classList.add('selected');
  // 全局最近选中（歌词面板等「动作上下文」使用），不代表当前页面的展示内容。
  state.current = t;
  const seq = ++detailSeq;
  const { data } = await api('/api/track/' + t.local_id);
  if (seq !== detailSeq) return; // 已有更新的选择，丢弃本次过期响应
  const trx = (data && data.track) || t;
  // 只写入「点击时所在页面」的选中态。
  state.viewSelection[view] = { track: t, trx: trx };
  // 该页内联详情条（纵向）。
  renderViewDetail(view);
  // 右侧独立详情栏是全局共享的：仅当用户仍停留在同一页面时才刷新。
  if (state.activeTab === view) renderActiveDetail();
}

// ==========================================================================
// 统一播放上下文（FR-16 ～ FR-18）
//
// 两类播放列表共用一套状态机：
//   * 有名播放列表 = DB 持久化的集合，播放时队列 = 列表全量（不受 n 影响）；
//   * 无名播放列表 = 播放瞬间由「触发播放的来源视图」推导出的 local_id[] 快照，
//     受 app 级 n 约束。播放是全局功能，不局限于播放列表页。
//
// 三条硬规则（防回归）：
//   1. 队列只快照 id 与显示摘要，**不引用** state.tracks 等来源数组
//      （搜索 / 扫描 / 导入 / 切 Tab 都不会改写正在播放的队列）。
//   2. n 只在「生成队列」这一处读取，绝不用于表格行数、DB 内容或导出内容。
//   3. 「仅选中、不播放」不切换队列；换来源播放一律重建当前队列。
// ==========================================================================

function freshPlayback() {
  return {
    queue: [],            // 实际播放顺序（洗牌后可能不同于界面顺序）
    originalQueue: [],    // 生成时的界面顺序快照（关闭随机时还原）
    meta: {},             // local_id → { name, artist } 显示摘要（副本，非引用）
    index: -1,
    source: '',
    title: '',
    named: false,         // true = 有名播放列表
    n: null,              // 生成队列时的 n；改 n 只影响下次生成
    shuffle: false,
    repeat: false,        // 列表循环
  };
}

// state 的队列字段（放在 state 初始化之后补齐，避免打乱既有初始化结构）。
state.playback = freshPlayback();

// #32：曲库表格由 library / favorites 两个一级视图共用（无二级栏）。
// tracksView() 决定选中态归属，libraryTracks() 决定渲染哪一组。
function tracksView() {
  return state.activeTab === 'favorites' ? 'favorites' : 'library';
}

function libraryTracks() {
  return (state.activeTab === 'favorites' ? state.favoriteTracks : state.tracks) || [];
}

function sourceTracks(view) {
  // v0.17.7：队列来源自动跳过不可访问曲目（避免"播放全部"塞满失效 id 反复 404）。
  // 例外：单独双击失效曲目走 playTrackToggleIfCurrent 的 verify 恢复入口，恢复后 t.unavailable
  // 已被清为 false，故此处过滤不影响恢复播放。
  const skipUnavailable = (arr) => (arr || []).filter((t) => !t.unavailable);
  if (view === 'favorites') return skipUnavailable(state.favoriteTracks);
  if (view === 'library') return skipUnavailable(state.tracks);
  if (view === 'playlists') return skipUnavailable(state.playlistTracks);
  if (view === 'albums') return skipUnavailable(state.albumTracks);
  if (view === 'artists') return skipUnavailable(state.artistTracks);
  // v0.12.7：曲源页三级曲目（按 source_kind 归属查询所得），供队列 / 双击播放使用。
  if (view === 'sources') return skipUnavailable(state.sourceTracks);
  return [];
}

// 「可视段」= 屏幕上实际可见的行；播放瞬间冻结成 [start, end)，播放过程中不再读 DOM。
function visibleRange(view, total) {
  if (!total) return [0, 0];
  const box = $(VIEW_BOX[view]);
  const table = box && box.querySelector('table.track-table');
  if (!table) return [0, Math.min(total, 1)];
  const rows = Array.from(table.querySelectorAll('tbody tr[data-id]'));
  if (!rows.length) return [0, Math.min(total, 1)];
  const host = box.getBoundingClientRect();
  let start = -1;
  let end = -1;
  rows.forEach((tr, i) => {
    const r = tr.getBoundingClientRect();
    if (r.bottom > host.top + 1 && r.top < host.bottom - 1) {
      if (start < 0) start = i;
      end = i + 1;
    }
  });
  if (start < 0) return [0, Math.min(total, 1)];
  return [Math.min(start, total - 1), Math.min(end, total)];
}

// FR-17 队列构造：可视段第一项为队首 → 向后最多 n 首 → 尾部不足从紧邻 V 之前连续段前补。
// 总数不足就不足（搜索过滤态不跨过滤补）；|V| > n 时以当前曲为锚在 V 内截连续 n 首。
function buildQueue(view, clickedId, named) {
  const src = sourceTracks(view);
  const ids = src.map((t) => Number(t.local_id)).filter((n) => n > 0);
  if (!ids.length) return null;
  if (named) return { queue: ids.slice(), n: null };  // 有名列表：全量，n 不生效

  const n = autoPlaylistLimit();                     // n 的唯一消费点
  const [vs, ve] = visibleRange(view, ids.length);
  const clickedIdx = ids.indexOf(clickedId);
  let from;
  let to;
  const vLen = ve - vs;
  if (vLen > n) {
    from = Math.max(vs, Math.min(clickedIdx - Math.floor(n / 2), ve - n));
    from = Math.min(Math.max(from, vs), Math.max(vs, ve - n));
    to = Math.min(from + n, ids.length);
  } else {
    from = vs;
    to = Math.min(from + n, ids.length);
  }
  // 尾部不足 → 从紧邻 V 之前的连续段前补（Q2.A），队列始终是 src 的连续区间。
  if (to - from < n && from > 0) {
    from = Math.max(0, from - (n - (to - from)));
  }
  // 兜底：确保被点击的曲一定在队列内（Q1 的根本要求）。
  if (clickedIdx >= 0 && (clickedIdx < from || clickedIdx >= to)) {
    from = Math.max(0, Math.min(clickedIdx, Math.max(0, ids.length - n)));
    to = Math.min(from + n, ids.length);
  }
  return { queue: ids.slice(from, to), n: n };
}

function queueTitle(view, named) {
  // #32：收藏是第 5 个一级视图，队列标题与曲库区分。
  if (view === 'favorites') return '喜爱';
  // FR-23：智能三视图（全部曲库 / 专辑 / 歌手）标题直接用维度名；
  // 有名播放列表仍属手工维护列表，标题不变。
  if (view === 'playlists' && state.currentPlaylist) {
    return '播放列表：' + state.currentPlaylist.name;
  }
  if (view === 'albums') {
    return '专辑：' + (state.currentAlbum ? state.currentAlbum.name : '未知专辑');
  }
  if (view === 'artists') {
    return '艺人：' + (state.currentArtist ? state.currentArtist.name : '未知艺人');
  }
  // v0.12.7：曲源页按归属形成的分组，队列标题带上曲源名。
  if (view === 'sources') {
    return '曲源：' + (state.currentSource ? state.currentSource.name
      : t('sources.local_library', { _fallback: '本地曲库' }));
  }
  return named ? '播放列表' : '全部曲库';
}

// 洗牌一次并固定顺序（S2）：整个队列 Fisher-Yates，随后把当前曲重新定位。
// 「上一首」按洗牌后的播放顺序回退，不再随机一次（S3）。
function applyShuffle() {
  const pb = state.playback;
  if (pb.queue.length < 2) return;
  const cur = pb.queue[pb.index];
  for (let i = pb.queue.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    const tmp = pb.queue[i];
    pb.queue[i] = pb.queue[j];
    pb.queue[j] = tmp;
  }
  pb.index = Math.max(0, pb.queue.indexOf(cur));
}

function restoreOriginalOrder() {
  const pb = state.playback;
  if (!pb.originalQueue.length) return;
  const cur = pb.queue[pb.index];
  pb.queue = pb.originalQueue.slice();
  pb.index = Math.max(0, pb.queue.indexOf(cur));
}

// 播放入口：所有来源（曲库 / 播放列表 / 专辑 / 歌手）都走这里重建上下文。
function startPlayback(t, view, opts) {
  const v = view || state.activeTab;
  if (!t || !Number(t.local_id)) return;
  const named = (v === 'playlists' && !!state.currentPlaylist);
  // FR-24 T1：双击二级列表整组播放是**独立新入口**，队列 = 来源视图当前全集。
  // 单曲双击 / 右键播放仍完全由 buildQueue() 决定（n / 视口段 V / 点击曲锚点），算法一行不改。
  const built = (opts && opts.fullGroup)
    ? { queue: sourceTracks(v).map((x) => Number(x.local_id)).filter((n) => n > 0), n: null }
    : buildQueue(v, Number(t.local_id), named);
  if (!built || !built.queue.length) { setHint('当前列表没有可播放的歌曲'); return; }

  // 只快照显示摘要，不引用来源数组（搜索 / 扫描 / 导入不会改写队列）。
  const meta = {};
  sourceTracks(v).forEach((x) => {
    const id = Number(x.local_id);
    if (id > 0) meta[id] = { name: x.name || '', artist: x.artist || '' };
  });
  const pb = freshPlayback();
  pb.shuffle = state.playback.shuffle;   // 模式是全局偏好，跨队列保留
  pb.repeat = state.playback.repeat;
  pb.originalQueue = built.queue.slice();
  pb.queue = built.queue.slice();
  pb.meta = meta;
  pb.source = v;
  pb.named = named;
  pb.n = built.n;
  pb.title = queueTitle(v, named);
  pb.index = Math.max(0, pb.queue.indexOf(Number(t.local_id)));
  state.playback = pb;
  if (pb.shuffle) applyShuffle();
  renderQueueUI();
  playCurrent();
}

// 兼容既有调用点：双击歌曲行 / 右键菜单「播放」都使用指定视图（或当前活动视图）作为队列来源。
// 列表内不再渲染 ▶ 按钮（用户要求）；播放入口统一为双击与右键菜单。
function playTrack(t, view) {
  startPlayback(t, view || state.activeTab);
}

// 双击列表歌曲：若双击的正是当前曲目（♪ 标记同源），则切换暂停/继续而非从头重播。
// v0.17.7：不可访问（⚠）曲目双击先做单点复核，恢复则清标并播放；失败保持标记 + 提示。
async function playTrackToggleIfCurrent(t, view) {
  if (t && t.unavailable) {
    const id = Number(t.local_id);
    const { ok, data } = await api('/api/tracks/' + id + '/verify', { method: 'POST' });
    if (!ok || !data || !data.accessible) {
      setHint('文件仍不可访问，未播放');
      return;
    }
    t.unavailable = false;
    // 同步所有内存视图里该曲的不可用状态，否则 sourceTracks() 过滤会把刚恢复的曲排除，
    // 导致双击恢复后 buildQueue 找不到锚点。
    ['tracks', 'playlistTracks', 'albumTracks', 'artistTracks', 'favoriteTracks', 'sourceTracks'].forEach((k) => {
      (state[k] || []).forEach((x) => { if (Number(x.local_id) === id) x.unavailable = false; });
    });
    // 增量更新所有视图的 ⚠ 标记（清掉该 id），不整表重建。
    document.querySelectorAll('.track-table .name-cell').forEach((cell) => {
      if (String(cell.dataset.localId) === String(id)) {
        cell.dataset.unavailable = 'false';
        const w = cell.querySelector('.unavailable-mark');
        if (w) w.remove();
      }
    });
    setHint('文件已恢复访问');
  }
  if (isPlayingTrack(t)) { window.kqkCommand('toggle-play'); return; }
  playTrack(t, view);
}

// v0.14.10：播放栏左列统一渲染——第一行曲名（播放时含艺人），第二行当前歌词行。
// 抽成函数供 playCurrent / clearQueue / restorePlaystate 复用，避免各处 textContent 覆盖
// 把第二行歌词节点一起抹掉；第二行文本由 syncLyrics() 随进度维护。
function setNowPlayingTitle(text) {
  const np = $('now-playing');
  np.textContent = '';
  np.appendChild(document.createTextNode(text || '未在播放'));
  const sub = document.createElement('span');
  sub.className = 'np-lyric';
  sub.textContent = '';
  np.appendChild(sub);
}

function playCurrent() {
  const pb = state.playback;
  const id = pb.queue[pb.index];
  if (!id) { setHint('播放列表已空'); return; }
  const m = pb.meta[id] || {};
  audio.src = '/api/audio/' + id;
  audio.play().catch(() => {});
  setNowPlayingTitle((m.name || ('#' + id)) + (m.artist ? ' — ' + m.artist : ''));
  // v0.17.7：按钮 ⏸/▶ 由 audio 的 play/pause/error 事件驱动（真实状态），
  // 歌词延迟到 playing 事件才拉（失效文件不发无效歌词请求）。
  // 这里不再直接写 btn-play 与 loadLyrics。
  // #29：全屏层打开时切曲，左栏标题/封面/副信息同步更新（零新增请求）。
  if (state.nowViewOpen) syncNowViewMeta();
  renderQueueUI();
  syncPlayingMark();
  savePlaystate();
}

// 上一首 / 下一首 / 自动推进。auto=true 表示由 audio.ended 触发。
function stepQueue(delta, auto) {
  const pb = state.playback;
  if (!pb.queue.length) { setHint('当前没有播放列表'); return; }
  let next = pb.index + delta;
  if (next < 0 || next >= pb.queue.length) {
    if (!pb.repeat) {
      if (auto) {
        $('btn-play').textContent = '▶';
        setHint('播放结束');
      } else {
        setHint(delta > 0 ? '已是播放列表最后一首' : '已是播放列表第一首');
      }
      return;
    }
    next = delta > 0 ? 0 : pb.queue.length - 1;   // 列表循环回绕
  }
  pb.index = next;
  playCurrent();
}

// 从来源视图补一首曲目的显示摘要（队列只存副本，绝不引用 state.tracks 的对象）。
function ensureQueueMeta(pb, id, view) {
  if (pb.meta[id]) return;
  const views = [view, 'library', 'playlists', 'albums', 'artists'];
  for (const v of views) {
    const hit = sourceTracks(v).find((x) => Number(x.local_id) === id);
    if (hit) {
      pb.meta[id] = { name: hit.name || '', artist: hit.artist || '' };
      return;
    }
  }
  pb.meta[id] = { name: '', artist: '' };
}

// 「下一首播放」插播（工具栏 / 队列面板 / 右键菜单共用）。
//
// FR-21 追加要求：被点歌曲必须**同步追加到当前无名播放列表**，成为队列的持久成员——
// 即同时写入 queue（播放顺序）与 originalQueue（界面顺序快照），并由 savePlaystate() 落盘，
// 关闭随机 / 重启后都仍在。当前没有队列时（例如未播放就右键），先按来源视图构造一个
// 无名列表再加入，而不是只提示「请先播放」。
function enqueueNext(id, view) {
  const pb = state.playback;
  const n = Number(id);
  if (!n) return;
  const v = view || pb.source || state.activeTab;

  // ① 尚无队列：按既有 n 规则建立无名播放列表，把该曲追加进去并开始播放。
  if (!pb.queue.length) {
    const built = buildQueue(v, n, false);
    const ids = built && built.queue.length ? built.queue.slice() : [];
    if (ids.indexOf(n) < 0) ids.push(n);          // 追加到当前无名播放列表
    const fresh = freshPlayback();
    fresh.shuffle = pb.shuffle;
    fresh.repeat = pb.repeat;
    fresh.originalQueue = ids.slice();
    fresh.queue = ids.slice();
    fresh.meta = {};
    ids.forEach((k) => ensureQueueMeta(fresh, k, v));
    fresh.source = v;
    fresh.named = false;
    fresh.n = built ? built.n : autoPlaylistLimit();
    fresh.title = queueTitle(v, false);
    fresh.index = Math.max(0, ids.indexOf(n));
    state.playback = fresh;
    if (fresh.shuffle) applyShuffle();
    renderQueueUI();
    playCurrent();                                // 内部会 savePlaystate()
    setHintOk('已建立自动播放列表，并把该曲加入其中');
    return;
  }

  // ② 已有队列：插播项同步进入两个快照，且**各自按自己的顺序定位**。
  // 随机模式下 queue 已被洗牌，若把洗牌后的下标 at 直接用到 originalQueue 会错位，
  // 关闭随机后插播项会跑到别的位置（旧缺陷）。
  const curId = pb.queue[pb.index];
  const pos = pb.queue.indexOf(n);
  if (pos >= 0) pb.queue.splice(pos, 1);
  const curIdx = Math.max(0, pb.queue.indexOf(curId));
  const at = Math.min(curIdx + 1, pb.queue.length);
  pb.queue.splice(at, 0, n);
  ensureQueueMeta(pb, n, v);

  const oPos = pb.originalQueue.indexOf(n);
  if (oPos >= 0) pb.originalQueue.splice(oPos, 1);
  const oCur = Math.max(0, pb.originalQueue.indexOf(curId));
  const oAt = Math.min(oCur + 1, pb.originalQueue.length);
  pb.originalQueue.splice(oAt, 0, n);

  pb.index = Math.max(0, pb.queue.indexOf(curId));
  renderQueueUI();
  savePlaystate();
  setHintOk('已插到下一首播放，并同步加入当前播放列表');
}

function clearQueue() {
  const shuffle = state.playback.shuffle;
  const repeat = state.playback.repeat;
  audio.pause();
  audio.removeAttribute('src');
  audio.load();
  state.playback = freshPlayback();
  state.playback.shuffle = shuffle;
  state.playback.repeat = repeat;
  setNowPlayingTitle('未在播放');
  $('btn-play').textContent = '▶';
  renderQueueUI();
  syncPlayingMark();
  api('/api/playstate', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ clear: true }),
  });
  setHintOk('播放列表已清空');
}

// 队列面板：超过窗口阈值时只渲染当前位置附近的一段，避免上千行 DOM 卡顿。
function renderQueueUI() {
  const pb = state.playback;
  const total = pb.queue.length;
  $('queue-drawer-title').textContent = pb.title || '播放列表';
  $('queue-count').textContent = total ? ((pb.index + 1) + '/' + total) : '';
  $('btn-shuffle').classList.toggle('is-active', pb.shuffle);
  $('btn-repeat').classList.toggle('is-active', pb.repeat);
  const WINDOW = 120;
  let from = 0;
  let to = total;
  if (total > WINDOW) {
    from = Math.max(0, pb.index - 40);
    to = Math.min(total, from + WINDOW);
    $('queue-count').textContent = ((pb.index + 1) + '/' + total + '（显示 ' + (from + 1) + '–' + to + '）');
  }
  const rows = [];
  for (let i = from; i < to; i++) {
    const id = pb.queue[i];
    const m = pb.meta[id] || {};
    rows.push(
      `<li data-id="${id}" class="${i === pb.index ? 'is-current' : ''}">` +
      `<span class="q-idx">${i + 1}</span>` +
      `<span class="q-name">${esc(m.name || ('#' + id))}</span>` +
      `<span class="q-artist">${esc(m.artist || '')}</span>` +
      `<button class="md-btn sm q-next" data-act="next" type="button">下一首播放</button>` +
      `</li>`,
    );
  }
  $('queue-list').innerHTML = rows.join('');
}

// 播放状态持久化：切歌 / 暂停 / 模式切换时写，不做定时落盘（S4）。
let playstateTimer = null;
function savePlaystate() {
  const pb = state.playback;
  if (!pb.queue.length) return;
  const payload = {
    queue: pb.queue,
    originalQueue: pb.originalQueue,
    meta: pb.meta,
    index: pb.index,
    source: pb.source,
    title: pb.title,
    named: pb.named,
    n: pb.n,
    shuffle: pb.shuffle,
    repeat: pb.repeat,
  };
  clearTimeout(playstateTimer);
  playstateTimer = setTimeout(() => {
    api('/api/playstate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
  }, 120);
}

// 启动恢复：队列与位置读回，但**不自动开始播放**（避免重启即出声）。
// 失效 id 由后端校验并丢弃整份队列（返回 state:null）。
async function restorePlaystate() {
  const { data } = await api('/api/playstate');
  const s = data && data.state;
  if (!s || !Array.isArray(s.queue) || !s.queue.length) return;
  const pb = freshPlayback();
  pb.queue = s.queue.map(Number);
  pb.originalQueue = (Array.isArray(s.originalQueue) && s.originalQueue.length
    ? s.originalQueue : s.queue).map(Number);
  pb.meta = s.meta && typeof s.meta === 'object' ? s.meta : {};
  pb.index = Math.max(0, Math.min(Number(s.index) || 0, pb.queue.length - 1));
  pb.source = s.source || '';
  pb.title = s.title || '播放列表';
  pb.named = !!s.named;
  pb.n = s.n == null ? null : Number(s.n);
  pb.shuffle = !!s.shuffle;
  pb.repeat = !!s.repeat;
  state.playback = pb;
  const id = pb.queue[pb.index];
  const m = pb.meta[id] || {};
  // v0.14.10：恢复态第二行由「已暂停提示」改为当前歌词行（标题用统一渲染，保留歌词节点）。
  // 用 textContent / createTextNode 写入，不拼 HTML，曲目名含特殊字符也不会被解析。
  setNowPlayingTitle(m.name || ('#' + id));
  // v0.14.10(a)：恢复队列即拉一次歌词，无需等用户点播放 / 打开歌词抽屉。
  if (id) loadLyrics(id);
  renderQueueUI();
  syncPlayingMark();
}

// ---------- 播放列表通用对话框（替代 prompt / confirm，避免原生对话框抢焦点）----------
let dialogResolve = null;
let dialogCancelValue = null;
function showDialog(opts) {
  return new Promise((resolve) => {
    dialogResolve = resolve;
    dialogCancelValue = Object.prototype.hasOwnProperty.call(opts, 'cancelValue') ? opts.cancelValue : null;
    $('pl-dialog-title').textContent = opts.title || '';
    $('pl-dialog-text').textContent = opts.text || '';
    const inp = $('pl-dialog-input');
    inp.hidden = !opts.input;
    inp.value = opts.value || '';
    const rep = $('pl-dialog-report');
    const report = opts.report || [];
    rep.hidden = !report.length;
    rep.innerHTML = report.map((x) => `<li>${esc(x)}</li>`).join('');
    $('pl-dialog-ok').textContent = opts.okText || '确定';
    $('pl-dialog-cancel').textContent = opts.cancelText || '取消';
    $('pl-dialog-cancel').hidden = !!report.length && !opts.input;
    $('pl-dialog-modal').hidden = false;
    if (opts.input) setTimeout(() => { inp.focus(); inp.select(); }, 0);
  });
}
function closeDialog(val) {
  $('pl-dialog-modal').hidden = true;
  const r = dialogResolve;
  dialogResolve = null;
  if (r) r(val);
}
$('pl-dialog-ok').addEventListener('click', () => {
  closeDialog($('pl-dialog-input').hidden ? true : $('pl-dialog-input').value);
});
$('pl-dialog-cancel').addEventListener('click', () => closeDialog(dialogCancelValue));
$('pl-dialog-input').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') closeDialog(e.target.value);
  if (e.key === 'Escape') closeDialog(dialogCancelValue);
});

// ==========================================================================
// 歌曲列表右键菜单（FR-21）
// 四张歌曲表（曲库 / 播放列表 / 专辑 / 歌手）共用一个浮动菜单。
// 菜单只保存 local_id 与曲目摘要快照，不引用表格行对象；播放列表清单异步补齐。
// 删除采用「移入曲库回收站 .mt/trash/ + 清索引」，不做不可逆删除（见后端 trash_track）。
// ==========================================================================
let ctxTrack = null;   // { id, track, view }

function ctxItem(icon, label, act, extra, danger) {
  return `<button type="button" class="ctx-item${danger ? ' ctx-danger' : ''}"` +
    ` data-ctx-act="${act}"${extra ? ' ' + extra : ''}${danger ? ' data-ctx-danger="1"' : ''}>` +
    `<span class="ctx-icon">${icon}</span><span class="ctx-label">${esc(label)}</span></button>`;
}

function renderTrackContextMenu(lists, view) {
  let html = ctxItem('▶', '播放', 'play') + ctxItem('⏭', '下一首播放', 'enqueue');
  html += '<div class="ctx-sep"></div><div class="ctx-group-title">添加到播放列表</div>';
  if (lists && lists.length) {
    lists.forEach((pl) => {
      html += ctxItem('＋', pl.name || '未命名列表', 'add', `data-pl-id="${Number(pl.id)}"`);
    });
  } else {
    html += '<div class="ctx-empty">暂无播放列表，可新建</div>';
  }
  html += ctxItem('＋', '新建播放列表并加入', 'new');
  if (view === 'playlists') {
    html += '<div class="ctx-sep"></div>' + ctxItem('⊖', '从本播放列表移除', 'remove');
  }
  html += '<div class="ctx-sep"></div>';
  html += ctxItem('📂', '打开文件所在位置', 'reveal') + ctxItem('⧉', '复制歌曲信息', 'copy');
  html += '<div class="ctx-sep"></div>' + ctxItem('🗑', '删除歌曲', 'delete', '', true);
  $('ctx-menu-items').innerHTML = html;
}

function positionTrackContextMenu(x, y) {
  const menu = $('track-context-menu');
  const w = menu.offsetWidth || 240;
  const h = menu.offsetHeight || 360;
  menu.style.left = Math.max(8, Math.min(x, window.innerWidth - w - 8)) + 'px';
  menu.style.top = Math.max(8, Math.min(y, window.innerHeight - h - 8)) + 'px';
}

function closeTrackContextMenu() {
  $('track-context-menu').hidden = true;
  ctxTrack = null;
}

async function openTrackContextMenu(e, t, view, tr) {
  if (!t || !(Number(t.local_id) > 0)) return;
  e.preventDefault();
  e.stopPropagation();
  const id = Number(t.local_id);
  ctxTrack = { id: id, track: t, view: view };
  // 右键即选中该行：菜单动作与详情面板上下文保持一致。
  selectTrack(t, tr);
  $('ctx-track-name').textContent = (t.name || '未命名') + (t.artist ? ' — ' + t.artist : '');
  renderTrackContextMenu([], view);
  $('track-context-menu').hidden = false;
  positionTrackContextMenu(e.clientX, e.clientY);
  const { data } = await api('/api/playlists');
  // 异步返回时菜单可能已关闭或已换目标，丢弃过期结果。
  if (!ctxTrack || ctxTrack.id !== id || $('track-context-menu').hidden) return;
  renderTrackContextMenu((data && data.playlists) || [], view);
  positionTrackContextMenu(e.clientX, e.clientY);
}

function ctxPost(path, body) {
  return api(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

async function runTrackContextAction(act, btn) {
  const ctx = ctxTrack;
  if (!ctx) return;
  const id = ctx.id;
  const t = ctx.track;
  const view = ctx.view;
  closeTrackContextMenu();

  if (act === 'play') { playTrack(t, view); return; }
  if (act === 'enqueue') { enqueueNext(id, view); return; }

  if (act === 'add' || act === 'new') {
    let plId = Number(btn.dataset.plId);
    if (act === 'new') {
      const name = await showDialog({
        title: '新建播放列表',
        text: `将把「${t.name || '未命名'}」加入新建的播放列表。`,
        input: true, value: '', okText: '创建并加入',
      });
      if (!name || !String(name).trim()) return;
      const created = await ctxPost('/api/playlists', { name: String(name).trim() });
      if (!created.data || !created.data.ok) {
        setHint((created.data && created.data.error) || '新建播放列表失败');
        return;
      }
      plId = Number(created.data.id);
      await loadPlaylists();
    }
    const added = await ctxPost('/api/playlists/' + plId + '/add', { local_id: id });
    if (!added.data || !added.data.ok) {
      setHint((added.data && added.data.error) || '加入播放列表失败');
      return;
    }
    await loadPlaylists();
    // 加入动作可能来自曲库 / 专辑 / 歌手任意视图：只要目标列表正是当前已打开的列表，
    // 就必须刷新右侧曲目表格，否则切回播放列表页看到的仍是旧快照。
    if (state.currentPlaylist && Number(state.currentPlaylist.id) === plId) {
      await loadPlaylistTracks(plId);
    }
    setHintOk(`「${t.name || '未命名'}」已加入播放列表`);
    return;
  }

  if (act === 'remove') { await removeFromPlaylist(id); return; }

  if (act === 'reveal') {
    const r = await ctxPost('/api/tracks/reveal', { local_id: id });
    setHint((r.data && r.data.ok) ? '已在资源管理器中定位文件' : ((r.data && r.data.error) || '定位失败'));
    return;
  }

  if (act === 'copy') {
    const text = (t.name || '未命名') + (t.artist ? ' — ' + t.artist : '') + (t.album ? '，' + t.album : '');
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        const ta = document.createElement('textarea');
        ta.value = text; ta.style.position = 'fixed'; ta.style.opacity = '0';
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        ta.remove();
      }
      setHintOk('已复制歌曲信息');
    } catch (err) {
      setHint('复制失败：' + (err && err.message ? err.message : err));
    }
    return;
  }

  if (act === 'delete') { await confirmDeleteTrack(id, t, view); }
}

async function confirmDeleteTrack(id, t, view) {
  const detail = await api('/api/track/' + id);
  const trx = (detail.data && detail.data.track) || t;
  const path = trx.file_path || '';
  const ok = await showDialog({
    title: '删除歌曲',
    text: `「${t.name || '未命名'}」将从曲库中删除。\n` +
      (path ? `文件：${path}\n` : '') +
      '\n文件移入曲库回收站 .mt/trash/（可手动取回），' +
      '索引与播放列表引用一并清除，不进入系统回收站。',
    okText: '移入回收站',
  });
  if (!ok) return;
  const res = await ctxPost('/api/tracks/delete', { local_id: id });
  if (!res.data || !res.data.ok) {
    setHint((res.data && res.data.error) || '删除失败');
    return;
  }
  dropFromPlaybackQueue(id);
  await refreshAfterTrackDelete(id, view);
  setHintOk(`已删除「${t.name || '未命名'}」` +
    (res.data.playlists_affected ? `，并从 ${res.data.playlists_affected} 个列表移除` : '') +
    (res.data.trashed ? '；文件在 .mt/trash/ 可恢复' : ''));
}

// 删除曲目后从正在播放的队列里摘掉该 id；若删的正是当前曲则顺延到下一首。
function dropFromPlaybackQueue(id) {
  const pb = state.playback;
  if (!pb.queue || !pb.queue.length || pb.queue.indexOf(id) < 0) return;
  const curId = pb.queue[pb.index];
  const wasPlaying = !audio.paused;
  pb.queue = pb.queue.filter((x) => x !== id);
  pb.originalQueue = (pb.originalQueue || []).filter((x) => x !== id);
  delete pb.meta[id];
  if (!pb.queue.length) { clearQueue(); return; }
  const kept = pb.queue.indexOf(curId);
  pb.index = kept >= 0 ? kept : Math.min(pb.index, pb.queue.length - 1);
  if (curId !== pb.queue[pb.index]) {
    if (wasPlaying) playCurrent();
    else {
      audio.pause(); audio.removeAttribute('src');
      $('btn-play').textContent = '▶';
      renderQueueUI(); savePlaystate();
    }
  } else {
    renderQueueUI(); savePlaystate();
  }
}

// 删除后刷新当前视图与分组计数，并清掉四页内存里残留的该曲，避免详情/队列显示脏数据。
async function refreshAfterTrackDelete(id, view) {
  ['tracks', 'playlistTracks', 'albumTracks', 'artistTracks', 'favoriteTracks', 'sourceTracks'].forEach((k) => {
    state[k] = (state[k] || []).filter((x) => Number(x.local_id) !== id);
  });
  ['library', 'playlists', 'albums', 'artists', 'favorites', 'sources'].forEach((v) => {
    const sel = state.viewSelection[v];
    if (sel && sel.track && Number(sel.track.local_id) === id) state.viewSelection[v] = null;
  });
  if (state.current && Number(state.current.local_id) === id) state.current = null;
  if (view === 'library') await loadTracks($('search').value);
  else if (view === 'favorites') await loadFavorites(true);
  else if (view === 'playlists' && state.currentPlaylist) await loadPlaylistTracks(state.currentPlaylist.id);
  else if (view === 'albums' && state.currentAlbum) await loadAlbumTracks(state.currentAlbum.name);
  else if (view === 'artists' && state.currentArtist) await loadArtistTracks(state.currentArtist.name);
  else if (view === 'sources' && state.currentSource) await loadSourceTracks(state.currentSource.id);
  await loadPlaylists();
  if (state.activeTab === 'albums') await loadAlbums();
  if (state.activeTab === 'artists') await loadArtists();
  // v0.12.7：替换归属修正后，曲源分组计数与曲目列表即时重算（从原列表移除、在新列表出现）。
  if (state.activeTab === 'sources') await loadSourceGroups();
  renderActiveDetail();
  renderViewDetail(state.activeTab);
}

$('ctx-menu-items').addEventListener('click', (e) => {
  const btn = e.target.closest('[data-ctx-act]');
  if (!btn) return;
  e.stopPropagation();
  runTrackContextAction(btn.dataset.ctxAct, btn);
});
document.addEventListener('click', (e) => {
  if (!e.target.closest('#track-context-menu')) closeTrackContextMenu();
});
document.addEventListener('contextmenu', (e) => {
  if (e.target.closest('#track-context-menu')) { e.preventDefault(); return; }
  closeTrackContextMenu();
});
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeTrackContextMenu(); });
window.addEventListener('resize', closeTrackContextMenu);
document.addEventListener('scroll', (e) => {
  // 菜单自身滚动（播放列表很多时）不能误关菜单。
  if (e.target === $('track-context-menu') ||
      (e.target.closest && e.target.closest('#track-context-menu'))) return;
  closeTrackContextMenu();
}, true);

// ---------- 歌词 ----------
// LRC 时间戳：[mm:ss] / [mm:ss.xx] / [mm:ss.xxx]，一行可带多个时间戳。
const LRC_STAMP = /\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]/g;

// 解析 LRC 文本 → { timed, lines:[{t,text}], plain }。
// 无时间轴时 timed=false，按纯文本整段展示。
function parseLrc(text) {
  const lines = [];
  let timed = false;
  const strip = new RegExp(LRC_STAMP.source, 'g');
  (text || '').split(/\r?\n/).forEach((raw) => {
    const line = raw.trim();
    if (!line) return;
    const stamps = [];
    const re = new RegExp(LRC_STAMP.source, 'g');
    let m;
    while ((m = re.exec(line)) !== null) {
      const min = parseInt(m[1], 10);
      const sec = parseInt(m[2], 10);
      // 小数部分按毫秒补齐（.5 → 500ms）。
      const frac = m[3] ? parseInt(m[3].padEnd(3, '0').slice(0, 3), 10) : 0;
      stamps.push(min * 60 + sec + frac / 1000);
    }
    if (!stamps.length) return; // 空行 / 元信息行（[ti:...]）不参与歌词轴。
    timed = true;
    const content = line.replace(strip, '').trim();
    stamps.forEach((t) => lines.push({ t, text: content }));
  });
  lines.sort((a, b) => a.t - b.t);
  return { timed, lines, plain: text || '' };
}

// 拉取并渲染歌词（local_id 为空则清空）。
async function loadLyrics(localId) {
  state.lyrics = null;
  state.lyricsIndex = -1;
  state.lyricsSource = '';
  state.lyricsLocalId = localId || null;
  // v0.14.10：切曲即清空播放栏第二行（新曲歌词到达前不残留上一首的歌词）。
  const npSub0 = $('now-playing') && $('now-playing').querySelector('.np-lyric');
  if (npSub0) npSub0.textContent = '';
  const srcEl = $('lyrics-source');
  const nvSrc = $('nv-src');
  if (srcEl) srcEl.textContent = '';
  if (nvSrc) nvSrc.textContent = '';
  if (!localId) { renderLyrics(); return; }
  const { data } = await api('/api/lyrics/' + localId);
  if (!data || !data.ok) { renderLyrics(); return; }
  const text = data.text || '';
  if (!text.trim()) { renderLyrics(); return; }
  state.lyrics = parseLrc(text);
  state.lyricsSource = data.source || '';
  const label = data.source === 'lrc' ? 'LRC 文件' : (data.source === 'embedded' ? '内嵌歌词' : '');
  if (srcEl) srcEl.textContent = label;
  if (nvSrc) nvSrc.textContent = label;
  renderLyrics();
  syncLyrics();
}

// #29：抽屉与全屏层共用同一份歌词 HTML，保证行序 / data-i / data-t 完全一致。
const LYRICS_BODIES = ['lyrics-body', 'nv-lyrics'];
function lyricsHtml(L) {
  if (!L) return '<div class="placeholder">暂无歌词</div>';
  if (!L.timed || !L.lines.length) {
    // 纯文本歌词：整段展示（去掉元信息行）。
    const plain = L.plain.split(/\r?\n/)
      .filter((l) => l.trim() && !/^\[[a-zA-Z]+:.*\]$/.test(l.trim()))
      .map((l) => `<div class="lyric-line">${esc(l.trim())}</div>`)
      .join('');
    return plain || '<div class="placeholder">暂无歌词</div>';
  }
  return L.lines
    .map((l, i) => `<div class="lyric-line" data-i="${i}" data-t="${l.t}">${esc(l.text) || '&nbsp;'}</div>`)
    .join('');
}

// 渲染歌词面板（抽屉 + 全屏层两个容器）。
function renderLyrics() {
  const html = lyricsHtml(state.lyrics);
  LYRICS_BODIES.forEach((id) => {
    const body = $(id);
    if (body) body.innerHTML = html;
  });
}

// 点击某行跳转到对应时间点：两个容器统一委托一次，避免逐行绑定。
document.addEventListener('click', (e) => {
  const line = e.target.closest && e.target.closest('.lyric-line[data-t]');
  if (!line || !line.closest('#lyrics-body, #nv-lyrics')) return;
  const t = parseFloat(line.dataset.t);
  if (!isNaN(t) && audio.duration) audio.currentTime = t;
});

// 按播放进度同步高亮行并居中滚动（抽屉 + 全屏层同步）。
function syncLyrics() {
  const L = state.lyrics;
  // v0.14.10：无歌词 / 纯文本歌词时，播放栏第二行清空（面板保持原有占位逻辑）。
  const npSub = $('now-playing') && $('now-playing').querySelector('.np-lyric');
  if (!L || !L.timed || !L.lines.length) {
    if (npSub) npSub.textContent = '';
    return;
  }
  const now = audio.currentTime;
  let idx = -1;
  for (let i = 0; i < L.lines.length; i++) {
    if (L.lines[i].t <= now + 0.05) idx = i;
    else break;
  }
  if (idx === state.lyricsIndex) return;
  state.lyricsIndex = idx;
  // v0.14.10：当前这句歌词同步到播放栏第二行（textContent 写入，歌词文本不做 HTML 解析）。
  if (npSub) npSub.textContent = idx < 0 ? '' : (L.lines[idx].text || '');
  LYRICS_BODIES.forEach((id) => {
    const body = $(id);
    if (!body) return;
    body.querySelectorAll('.lyric-line.current').forEach((el) => el.classList.remove('current'));
    if (idx < 0) return;
    const el = body.querySelector(`.lyric-line[data-i="${idx}"]`);
    if (!el) return;
    el.classList.add('current');
    // 隐藏容器 clientHeight=0，跳过滚动；打开全屏层时会强制重算一次。
    if (body.clientHeight > 0) {
      body.scrollTop = el.offsetTop - body.clientHeight / 2 + el.offsetHeight / 2;
    }
  });
}

// 歌词面板开关（按钮激活态与抽屉显示态同步）。
$('btn-lyrics').addEventListener('click', () => {
  const drawer = $('lyrics-drawer');
  drawer.hidden = !drawer.hidden;
  $('btn-lyrics').classList.toggle('is-active', !drawer.hidden);
  if (!drawer.hidden) {
    const cur = state.current;
    if (cur) {
      $('lyrics-title').textContent = cur.name + ' — ' + (cur.artist || '');
      loadLyrics(cur.local_id);
    }
  }
});
$('lyrics-close').addEventListener('click', () => {
  $('lyrics-drawer').hidden = true;
  $('btn-lyrics').classList.remove('is-active');
});

// ---------- #29 正在播放 / 全屏歌词 ----------
// 元数据只从内存取（播放队列 meta + 各视图已加载的曲目数组），零新增 API 端点。
function nowViewTrack(id) {
  const pools = [state.tracks, state.playlistTracks, state.albumTracks, state.artistTracks];
  for (const arr of pools) {
    if (!Array.isArray(arr)) continue;
    const hit = arr.find((t) => Number(t.local_id) === Number(id));
    if (hit) return hit;
  }
  return null;
}

function syncNowViewMeta() {
  const pb = state.playback;
  const id = pb.queue[pb.index];
  const m = pb.meta[id] || null;
  const titleEl = $('nv-title');
  const metaEl = $('nv-meta');
  const coverEl = $('nv-cover');
  if (!titleEl || !coverEl) return;
  if (!m) {
    titleEl.textContent = '未在播放';
    if (metaEl) metaEl.textContent = '';
    coverEl.innerHTML = '<span class="nv-fallback" aria-hidden="true">♫</span>';
    return;
  }
  const t = nowViewTrack(id);
  titleEl.textContent = m.name || ('#' + id);
  if (metaEl) metaEl.textContent = [m.artist, t && t.album, t && t.format].filter(Boolean).join('，');
  // fallback 常驻、被 img 盖住；/api/cover 返回 204/404 时只隐藏 img、不删节点（同 #27）。
  coverEl.innerHTML = `<img src="/api/cover/${Number(id)}" alt="" />` +
    '<span class="nv-fallback" aria-hidden="true">♫</span>';
}

function openNowView() {
  const view = $('now-view');
  if (!view) return;
  state.nowViewOpen = true;
  view.hidden = false;
  $('btn-now-view').classList.add('is-active');
  syncNowViewMeta();
  const id = state.playback.queue[state.playback.index];
  if (id && state.lyricsLocalId === id) {
    // 歌词已是当前曲目：复用内存数据，不重复请求 /api/lyrics。
    renderLyrics();
    state.lyricsIndex = -1;
    syncLyrics();
  } else if (id) {
    loadLyrics(id);
  } else {
    renderLyrics();
  }
}

function closeNowView() {
  const view = $('now-view');
  if (!view) return;
  state.nowViewOpen = false;
  view.hidden = true;
  $('btn-now-view').classList.remove('is-active');
}

$('btn-now-view').addEventListener('click', () => {
  if (state.nowViewOpen) closeNowView();
  else openNowView();
});
$('nv-close').addEventListener('click', closeNowView);
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && state.nowViewOpen) closeNowView();
});

// ---------- 播放控制（全局队列状态机）----------
$('btn-play').addEventListener('click', () => {
  if (!audio.src) {
    // 重启后恢复了队列但尚未设置 audio.src：点播放即从恢复的位置继续。
    if (state.playback.queue.length) { playCurrent(); return; }
    return;
  }
  if (audio.paused) { audio.play().catch(() => {}); $('btn-play').textContent = '⏸'; }
  else { audio.pause(); $('btn-play').textContent = '▶'; }
});
audio.addEventListener('timeupdate', () => {
  if (audio.duration) {
    $('seek').value = Math.floor((audio.currentTime / audio.duration) * 1000);
    // v0.12.15：顺带把进度百分比写成 CSS 变量，供渐变轨道着色（不改播放逻辑）。
    $('seek').style.setProperty('--p', ((audio.currentTime / audio.duration) * 100).toFixed(2) + '%');
    $('cur-time').textContent = fmtTime(audio.currentTime * 1000);
  }
  syncLyrics();
});
audio.addEventListener('loadedmetadata', () => {
  $('dur-time').textContent = fmtTime(audio.duration * 1000);
});
// 一曲结束：按队列推进；列表循环开启时回绕，否则停在末尾。
audio.addEventListener('ended', () => { stepQueue(1, true); });
// 暂停时落盘（S4：不做定时写盘）。
audio.addEventListener('pause', () => { savePlaystate(); });
// 文件失效 / 无法解码：标记不可访问 + 从队列剔除 + 跳过该 id（FR-18 / v0.17.7）。
let errorSkips = 0;
audio.addEventListener('error', async () => {
  // 无头断言环境里 /api/audio 不可达，禁用自动跳过以免队列被连续推进（仅供测试置位）。
  if (window.__KQK_DISABLE_AUDIO_ERROR_SKIP__) return;
  const pb = state.playback;
  if (!audio.src || !pb.queue.length) return;
  const failedId = pb.queue[pb.index];
  errorSkips += 1;

  // v0.17.7：后端单点复核并打标（口径：metadata 成功且非空）；随后从队列剔除，
  // 避免前后切歌又撞上同一个失效 id。verify 失败（网络抖动）也照常剔除，不影响播放。
  if (failedId != null) {
    try { await api('/api/tracks/' + failedId + '/verify', { method: 'POST' }); } catch (_) {}
    const pos = pb.queue.indexOf(Number(failedId));
    if (pos >= 0) {
      pb.queue.splice(pos, 1);
      if (pb.originalQueue) {
        const oi = pb.originalQueue.indexOf(Number(failedId));
        if (oi >= 0) pb.originalQueue.splice(oi, 1);
      }
      if (pb.meta) delete pb.meta[Number(failedId)];
      pb.index = Math.min(pos, Math.max(0, pb.queue.length - 1));
      syncTrackMarks();
      savePlaystate();
    }
  }

  if (!pb.queue.length) {
    setHint('曲目文件不可访问，已从播放队列移除');
    errorSkips = 0;
    $('btn-play').textContent = '▶';
    return;
  }
  if (errorSkips > pb.queue.length + 1) {
    setHint('播放列表中多首歌曲已失效，播放停止');
    errorSkips = 0;
    $('btn-play').textContent = '▶';
    return;
  }
  setHint('曲目文件不可访问，已从播放队列移除');
  playCurrent();
});
audio.addEventListener('playing', () => {
  errorSkips = 0;
  $('btn-play').textContent = '⏸';
  const id = currentPlayingId();
  if (id != null) loadLyrics(id);
});
audio.addEventListener('play', () => { $('btn-play').textContent = '⏸'; });
audio.addEventListener('pause', () => { $('btn-play').textContent = '▶'; });
$('seek').addEventListener('input', () => {
  if (audio.duration) audio.currentTime = audio.duration * ($('seek').value / 1000);
});

// 交互增强 b 级：播放状态变化 → 推送托盘 + 更新系统媒体键元数据（事件驱动，零轮询）。
audio.addEventListener('play', () => { notifyTrayState(); updateMediaSessionPlaybackState('playing'); });
audio.addEventListener('pause', () => { notifyTrayState(); updateMediaSessionPlaybackState('paused'); });
audio.addEventListener('loadedmetadata', () => { updateMediaSessionMetadata(); notifyTrayState(); });
audio.addEventListener('ended', () => { notifyTrayState(); });

// 当前曲目元信息（标题 / 歌手），优先播放队列 meta，回退到选中曲目。
function currentTrackMeta() {
  const pb = state.playback;
  const id = pb && pb.queue ? pb.queue[pb.index] : undefined;
  const m = (id !== undefined && pb.meta) ? (pb.meta[id] || {}) : {};
  const t = state.current || {};
  return { title: m.name || t.name || '', artist: m.artist || t.artist || '' };
}

// 推送托盘状态（事件驱动，零轮询）。
function notifyTrayState() {
  const meta = currentTrackMeta();
  api('/api/tray/state', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      title: meta.title,
      artist: meta.artist,
      playing: !audio.paused && !audio.ended,
      muted: !!state.muted,
      has_track: !!state.current,
    }),
  }).catch(() => {});
}

// 系统媒体键 / 锁屏控件（无 artwork，避免加载失败）。
function updateMediaSessionMetadata() {
  if (!('mediaSession' in navigator)) return;
  const meta = currentTrackMeta();
  navigator.mediaSession.metadata = new MediaMetadata({
    title: meta.title || '未在播放',
    artist: meta.artist || 'K music',
    album: 'K music',
  });
}
function updateMediaSessionPlaybackState(s) {
  if (!('mediaSession' in navigator)) return;
  navigator.mediaSession.playbackState = s;
}
function setupMediaSession() {
  if (!('mediaSession' in navigator)) return;
  const map = {
    play: () => window.kqkCommand('toggle-play'),
    pause: () => window.kqkCommand('toggle-play'),
    previoustrack: () => window.kqkCommand('prev'),
    nexttrack: () => window.kqkCommand('next'),
    stop: () => { audio.pause(); },
  };
  Object.keys(map).forEach((action) => {
    try { navigator.mediaSession.setActionHandler(action, map[action]); } catch (e) { /* 不支持的动作忽略 */ }
  });
}

$('btn-prev').addEventListener('click', () => stepQueue(-1, false));
$('btn-next').addEventListener('click', () => stepQueue(1, false));
$('btn-shuffle').addEventListener('click', () => {
  const pb = state.playback;
  pb.shuffle = !pb.shuffle;
  if (pb.shuffle) applyShuffle(); else restoreOriginalOrder();
  renderQueueUI();
  savePlaystate();
  setHintOk(pb.shuffle ? '随机播放：已打乱当前播放列表并固定顺序' : '顺序播放：已恢复界面顺序');
});
$('btn-repeat').addEventListener('click', () => {
  const pb = state.playback;
  pb.repeat = !pb.repeat;
  renderQueueUI();
  savePlaystate();
  setHintOk(pb.repeat ? '列表循环已开启' : '列表循环已关闭（顺序播放到末尾停止）');
});
$('btn-queue').addEventListener('click', () => {
  const box = $('queue-drawer');
  box.hidden = !box.hidden;
  if (!box.hidden) renderQueueUI();
});
$('queue-close').addEventListener('click', () => { $('queue-drawer').hidden = true; });
$('queue-clear').addEventListener('click', () => clearQueue());
$('queue-list').addEventListener('click', (e) => {
  const li = e.target.closest('li[data-id]');
  if (!li) return;
  const id = Number(li.dataset.id);
  if (e.target.closest('[data-act="next"]')) { enqueueNext(id); return; }
  const pb = state.playback;
  pb.index = Math.max(0, pb.queue.indexOf(id));
  playCurrent();
});
// FR-29：主界面顶部「下一首播放」按钮已移除；enqueueNext() 仍由右键菜单与队列面板调用，
// FR-21「同步写 queue/originalQueue + savePlaystate()」不变式不动。

// ---------- 顶栏「重新扫描曲库」（原工具栏「扫描曲库」迁移，逻辑不变） ----------
$('btn-rescan').addEventListener('click', async () => {
  setHint('扫描中…');
  const { data } = await api('/api/scan', { method: 'POST' });
  const scanOk = !!(data && data.ok);
  let msg = scanOk
    ? `曲库扫描：新增 ${data.inserted}，更新 ${data.updated}，移除 ${data.removed}${data.held ? `，重复挂起 ${data.held}` : ''}`
    : '曲库扫描失败';
  // v0.16.11：串联外部曲源导入，让「重新扫描曲库」一并覆盖 [[sources]] 上游目录。
  // boot 不预加载 state.sources，必须先拉取；沿用各曲源 copy/move 配置（网易云后端强制 copy）。
  if (scanOk) {
    try {
      await loadSources();
      const enabled = (state.sources || []).filter((s) => s.enabled);
      if (enabled.length) {
        await updateAllEnabledSources();
        const ss = state.sourceSummary || {};
        msg += `；曲源导入：已完成 ${ss.done || 0}/${ss.total || 0}，入库 ${ss.imported || 0}，跳过 ${ss.skipped || 0}，筛除 ${ss.filtered || 0}，同源跳过 ${ss.intra || 0}，升级 ${ss.upgraded || 0}，低质忽略 ${ss.low || 0}`;
      } else {
        msg += '；无启用的曲源';
      }
    } catch (e) {
      msg += '；曲源导入未完成';
    }
  }
  setHint(msg);
  // 手动扫描是新决策点：即使之前忽略过，发现散落 .ncm 仍再次询问。
  maybeShowNcmOrganize((data && data.ncm_found) || [], true);
  // FR-22 P1①：手动「重新扫描曲库」才打开批量对比窗；boot scan 只静默登记角标。
  if (scanOk && (data.pending_duplicates || 0) > 0) {
    updateDupBadge(data.pending_duplicates);
    await openDuplicatesDialog();
  } else {
    await refreshDuplicateBadge();
  }
  loadTracks($('search').value);
});

// ==========================================================================
// FR-22 重复文件挂起与批量对比决策（W15 角标 / W4 全部未决组一次展示 / C 语义预选）
// 交互范式复用 loose_ncm：状态里带数据 + 用户主动点才弹窗，启动不打断。
// ==========================================================================
const DUP_ORIGIN_LABELS = {
  'netease-sync': '网易云同步', 'ncm-backfill': 'NCM 回补',
  manual: '手工导入', scan: '曲库扫描',
};

function updateDupBadge(n) {
  const count = Number(n) || 0;
  state.pendingDuplicates = count;
  // v0.12.2：顶栏入口改纯图标，未决状态只显示无数字小圆点；组数仍写在悬浮提示里，
  // 来源管理弹窗左侧沿用数字胶囊，数量信息不丢失。
  const dot = $('sources-badge');
  if (dot) dot.hidden = count === 0;
  const nav = $('sources-nav-badge');
  if (nav) {
    nav.textContent = String(count);
    nav.hidden = count === 0;
  }
  const btn = $('btn-sources');
  if (btn) {
    const label = (window.KQK && typeof window.KQK.t === 'function')
      ? window.KQK.t('sources.btn', { _fallback: '曲源' })
      : '曲源';
    btn.setAttribute('aria-label', label);
    btn.title = count
      ? `${count} 组待处理重复文件，点开「${label}」处理`
      : `${label}（暂无待处理重复文件）`;
  }
  const sub = $('sources-dup-sub');
  if (sub) sub.textContent = count ? `${count} 组未决` : '逐组对比与决策';
}

/** 轻量刷新：只读 /api/status 的角标字段，不重置外观 / 列表 / 选中态。
 *  boot scan 在后台跑，启动瞬间的 status 可能还没登记完，故定时补拉。 */
async function refreshDuplicateBadge() {
  const { data } = await api('/api/status');
  if (data) {
    state.duplicatesDir = data.pending_duplicates_dir || state.duplicatesDir;
    updateDupBadge(data.pending_duplicates);
  }
}

function dupAttrsHtml(m) {
  const rows = [
    ['格式', m.format ? String(m.format).toUpperCase() : ''],
    ['采样率', m.sample_rate ? (Number(m.sample_rate) / 1000).toFixed(1) + ' kHz' : ''],
    ['位深', m.bit_depth ? m.bit_depth + ' bit' : ''],
    ['码率', m.bitrate ? Math.round(Number(m.bitrate) / 1000) + ' kbps' : ''],
    ['大小', (m.file_size || m.size) ? fmtSize(m.file_size || m.size) : ''],
    // v0.14.0：不同质量版本需展示「位置 + 名称 + 大小」供用户选留哪个。
    ['位置', m.file_path || m.source || ''],
    ['来源', m.origin ? (DUP_ORIGIN_LABELS[m.origin] || m.origin) : ''],
  ].filter(([, v]) => v !== '' && v != null);
  if (!rows.length) return '';
  return '<div class="dup-option-attrs">' +
    rows.map(([k, v]) => `<span>${k}</span><b title="${esc(v)}">${esc(v)}</b>`).join('') + '</div>';
}

function dupOptionHtml(g, choice, name, meta, previewUrl, checked) {
  const origin = meta.origin || g.origin;
  return `<label class="dup-option${checked ? ' is-selected' : ''}">` +
    `<span class="dup-option-top">` +
      `<input type="radio" name="dup-${esc(g.id)}" value="${esc(choice)}" data-group-id="${esc(g.id)}"${checked ? ' checked' : ''} />` +
      `<span class="dup-option-name">${esc(name)}</span>` +
    `</span>` +
    dupAttrsHtml(Object.assign({}, meta, { origin })) +
    `<button type="button" class="md-btn sm dup-preview-btn" data-preview-url="${esc(previewUrl)}">▶ 试听</button>` +
  `</label>`;
}

/** 渲染重复对比组。v0.12.0 起支持传入容器，来源管理右侧与原对比弹窗共用同一份渲染。 */
function renderDuplicateGroups(list, box = $('dup-groups')) {
  box.innerHTML = '';
  if (!list.length) {
    box.innerHTML = '<div class="dup-empty">没有待处理的重复组</div>';
    return;
  }
  list.forEach((g, gi) => {
    const choices = [];
    if (g.existing) choices.push('existing');
    (g.candidates || []).forEach((c) => choices.push(c.id));
    // C 语义：命中历史决策 → 默认预选（仍弹窗、仍可改选），否则默认保留库内那份。
    let selected = g.suggested;
    if (!choices.includes(selected)) selected = g.existing ? 'existing' : choices[0];
    const options = [];
    if (g.existing) {
      options.push(dupOptionHtml(
        g, 'existing', '曲库内的版本', g.existing,
        `/api/audio/${encodeURIComponent(g.existing.local_id)}`, selected === 'existing'
      ));
    }
    (g.candidates || []).forEach((c, ci) => {
      const label = (c.source || '').split(/[\\/]/).pop() || `新文件 ${ci + 1}`;
      options.push(dupOptionHtml(
        g, c.id, `新文件 ${ci + 1}：${label}`, c,
        `/api/duplicates/preview/${encodeURIComponent(c.id)}`, selected === c.id
      ));
    });
    const div = document.createElement('div');
    div.className = 'dup-group';
    div.innerHTML =
      `<div class="dup-group-title">${gi + 1}. ${esc(g.title || '未知曲目')}</div>` +
      `<div class="dup-group-meta">${esc(g.artist || '')}，${esc((g.format || '').toUpperCase())}，${options.length} 个版本，${esc(DUP_ORIGIN_LABELS[g.origin] || g.origin || '')}</div>` +
      (g.last_choice ? `<div class="dup-last">${esc(g.last_choice)}（已预选，可调整）</div>` : '') +
      (!g.existing ? '<div class="dup-last">该组均为新文件，默认选中第一份</div>' : '') +
      `<div class="dup-options">${options.join('')}</div>`;
    box.appendChild(div);
  });
}

let dupPreviewBtn = null;
function previewDuplicate(url, btn) {
  const a = $('dup-preview-audio');
  if (!a) return;
  if (dupPreviewBtn && dupPreviewBtn !== btn) dupPreviewBtn.textContent = '▶ 试听';
  if (!a.paused && a.dataset.url === url) {
    a.pause();
    btn.textContent = '▶ 试听';
    dupPreviewBtn = null;
    return;
  }
  a.dataset.url = url;
  a.src = url;
  dupPreviewBtn = btn;
  btn.textContent = '⏸ 停止试听';
  const played = a.play();
  if (played && typeof played.catch === 'function') {
    played.catch(() => { btn.textContent = '▶ 试听'; dupPreviewBtn = null; });
  }
}

function stopDuplicatePreview() {
  const a = $('dup-preview-audio');
  if (a) { a.pause(); a.removeAttribute('src'); a.load(); delete a.dataset.url; }
  if (dupPreviewBtn) dupPreviewBtn.textContent = '▶ 试听';
  dupPreviewBtn = null;
}

async function openDuplicatesDialog() {
  const { data } = await api('/api/duplicates');
  if (!data || !data.ok) {
    setHint((data && data.error) || '读取重复组失败');
    return;
  }
  const list = data.groups || [];
  state.duplicateGroups = list;
  state.duplicatesDir = data.pending_dir || state.duplicatesDir;
  state.duplicatesLogPath = data.log_path || '';
  updateDupBadge(data.count == null ? list.length : data.count);
  if (!list.length) {
    setHint('暂无待处理的重复文件');
    return;
  }
  $('dup-count').textContent = `共 ${list.length} 组待处理（全部未决组一次展示）`;
  $('dup-path').textContent = state.duplicatesDir
    ? `挂起文件：${state.duplicatesDir}（暂不处理不会移动文件；换曲库时随曲库保留）` : '';
  renderDuplicateGroups(list);
  $('duplicates-modal').hidden = false;
}

function closeDuplicatesDialog() {
  stopDuplicatePreview();
  $('duplicates-modal').hidden = true;
}

/** 提交重复决策。prefix='dup' 为原对比弹窗，'src-dup' 为来源管理内嵌页，共用同一决策后端。 */
async function submitDuplicateDecisions(prefix = 'dup') {
  const inSources = prefix === 'src-dup';
  const groupsId = inSources ? 'src-dup-groups' : 'dup-groups';
  const applyId = inSources ? 'src-dup-apply' : 'dup-apply';
  const inputs = Array.from(document.querySelectorAll(`#${groupsId} input[type="radio"]:checked`));
  const decisions = inputs.map((el) => ({ group_id: el.dataset.groupId, choice: el.value }))
    .filter((d) => d.group_id && d.choice);
  if (!decisions.length) { setHint('请先为每组选择要保留的版本'); return; }
  const btn = $(applyId);
  btn.disabled = true;
  stopDuplicatePreview();
  const { data } = await api('/api/duplicates/decide', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ decisions }),
  });
  btn.disabled = false;
  if (!data || !data.ok) {
    setHint((data && data.error) || '提交决策失败');
    return;
  }
  const failed = data.failed || [];
  if (failed.length) setHint(`部分决策未生效：${failed.join('；')}`);
  else setHintOk(`已处理 ${data.applied} 组重复文件`);
  updateDupBadge(data.pending_duplicates);
  // v0.12.7：替换决策可能修正归属，曲源分组即时重算（只 UPDATE 同一行，local_id 不变）。
  if (state.activeTab === 'sources') await loadSourceGroups();
  if ((Number(data.pending_duplicates) || 0) > 0) {
    // 仍有未决组（决策失败 / 别的批次）：保留当前界面并重新拉取。
    if (inSources) await loadSourcesDuplicates();
    else await openDuplicatesDialog();
  } else if (inSources) {
    renderDuplicateGroups([], $(groupsId));
    $(inSources ? 'src-dup-count' : 'dup-count').textContent = '暂无待处理的重复组';
  } else {
    closeDuplicatesDialog();
  }
  await refreshDuplicateBadge();
  loadTracks($('search').value);
}

// v0.12.11：顶栏「曲源总览」入口改为打开设置并落到「曲源总览」子页（不弹独立窗口）。
$('btn-sources').addEventListener('click', openSourcesPage);
$('dup-later').addEventListener('click', () => {
  // W4：稍后处理不提交决策，挂起文件保持留在 .mt/pending-duplicates/。
  closeDuplicatesDialog();
  setHint(state.pendingDuplicates
    ? `已保留 ${state.pendingDuplicates} 组待处理重复文件，可随时点顶栏「曲源」继续` : '已关闭对比窗口');
});
$('dup-apply').addEventListener('click', () => submitDuplicateDecisions('dup'));
$('src-dup-later').addEventListener('click', () => {
  // v0.12.11：待处理重复是设置子页的一块，「稍后处理」不提交决策，只关闭设置面板。
  stopDuplicatePreview();
  $('settings-modal').hidden = true;
});
$('src-dup-apply').addEventListener('click', () => submitDuplicateDecisions('src-dup'));

/** change / click 事件委托：两个重复容器共用（选中态高亮 + 试听）。 */
function bindDuplicateGroups(box) {
  if (!box) return;
  box.addEventListener('change', (e) => {
    const input = e.target;
    if (!input.matches || !input.matches('input[type="radio"]')) return;
    const wrap = input.closest('.dup-options');
    if (!wrap) return;
    wrap.querySelectorAll('.dup-option').forEach((el) =>
      el.classList.toggle('is-selected', el.contains(input)));
  });
  box.addEventListener('click', (e) => {
    const btn = e.target.closest && e.target.closest('[data-preview-url]');
    if (btn) { e.preventDefault(); previewDuplicate(btn.dataset.previewUrl, btn); }
  });
}
bindDuplicateGroups($('dup-groups'));
bindDuplicateGroups($('src-dup-groups'));
$('dup-preview-audio').addEventListener('ended', () => {
  if (dupPreviewBtn) dupPreviewBtn.textContent = '▶ 试听';
  dupPreviewBtn = null;
});
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  if (!$('duplicates-modal').hidden) { closeDuplicatesDialog(); return; }
  if (!$('init-wizard').hidden) return;              // 向导不可 Esc 关闭
  if (!$('sources-modal').hidden) { closeSourceAddModal(); return; }
  if (!$('import-modal').hidden) { $('import-modal').hidden = true; return; }
  if (!$('settings-modal').hidden) $('settings-modal').hidden = true;
});
$('set-dup-clear-history').addEventListener('click', async () => {
  const yes = await showDialog({
    title: '清除历史',
    text: '清空同步与导入流水，重复对比不再提供上次选择的预选；挂起的重复组不受影响。',
    okText: '清除',
  });
  if (!yes) return;
  const { data } = await api('/api/duplicates/history/clear', { method: 'POST' });
  setHint(data && data.ok ? '同步/导入历史已清除' : ((data && data.error) || '清除历史失败'));
  if (data && data.ok) {
    state.dupHistoryCleared = true;
    updateHistorySummary();
  }
});

/** v0.14.11：同步/导入历史摘要（默认折叠）。无后端条数 API，按清除状态给出诚实文案。 */
function updateHistorySummary() {
  const el = $('src-history-summary-text');
  if (!el) return;
  el.textContent = state.dupHistoryCleared
    ? '暂无历史记录（上次选择已清除）'
    : '重复对比的上次选择记录（本地 .mt/sync-import.jsonl）';
}
$('src-history-toggle').addEventListener('click', () => {
  const btn = $('src-history-toggle');
  const body = $('src-history-body');
  const open = btn.getAttribute('aria-expanded') !== 'true';
  btn.setAttribute('aria-expanded', open ? 'true' : 'false');
  body.hidden = !open;
  syncSrcBlockLayout();
});
updateHistorySummary();

// ---------- 散落 .ncm 处理（扫描发现 → 询问 → 移入 download 按下载逻辑处理）----------
function maybeShowNcmOrganize(paths, force) {
  const list = (paths || []).filter((p) => force || !state.ncmDismissed.has(p));
  if (!list.length) return;
  state.ncmOrganizePaths = list;
  const ul = $('ncm-organize-list');
  ul.innerHTML = '';
  list.forEach((p, i) => {
    const name = p.split(/[\\/]/).pop() || p;
    const li = document.createElement('li');
    li.innerHTML =
      `<span class="ncm-idx">${i + 1}</span>` +
      `<span class="ncm-name">${esc(name)}</span>` +
      `<span class="ncm-path">${esc(p)}</span>`;
    ul.appendChild(li);
  });
  $('ncm-organize-count').textContent = `共发现 ${list.length} 个散落的 .ncm 文件`;
  $('ncm-organize-modal').hidden = false;
}

$('ncm-organize-cancel').addEventListener('click', () => {
  for (const p of state.ncmOrganizePaths) state.ncmDismissed.add(p);
  $('ncm-organize-modal').hidden = true;
});

$('ncm-organize-go').addEventListener('click', async () => {
  const btn = $('ncm-organize-go');
  btn.disabled = true;
  setHint('正在移入 download 并处理散落 .ncm…');
  // 批量转换：打开进度面板，逐首显示「正在转换哪一首」。
  showProgressPanel('散落 .ncm 整理');
  startProgressPolling();
  const { data } = await api('/api/ncm/organize', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ paths: state.ncmOrganizePaths }),
  });
  finishProgressTask();
  btn.disabled = false;
  if (!data || !data.ok) {
    setHint((data && data.error) || '处理散落 .ncm 失败');
    return;
  }
  $('ncm-organize-modal').hidden = true;
  const parts = [`入库 ${data.imported}`];
  if (data.pending) parts.push(`暂存 ${data.pending}（无工具，待配置后自动转换）`);
  if (data.skipped) parts.push(`跳过 ${data.skipped}`);
  if (data.failed) parts.push(`失败 ${data.failed}`);
  setHintOk(`散落 .ncm 处理完成：${parts.join('，')}`);
  await refreshStatus();
  loadTracks($('search').value);
});
// v0.16.2：搜索 180ms 防抖，只在此处（不进 loadTracks 本体），重试 / 切页等即时调用不受影响。
$('search').addEventListener('input', (e) => {
  const q = e.target.value;
  if (searchDebounce) clearTimeout(searchDebounce);
  searchDebounce = setTimeout(() => { searchDebounce = null; loadTracks(q); }, 180);
});
// 失败态重试：直接重发当前 query 与筛选，不清空旧列表。
document.addEventListener('click', (e) => {
  const btn = e.target && e.target.closest ? e.target.closest('#track-load-retry') : null;
  if (!btn) return;
  e.stopPropagation();
  loadTracks(state.tracksQuery);
});

// ---------- 左侧 tab 选择栏（全部曲库 / 播放列表 / 专辑 / 歌手 / 喜爱）----------
// #32：收藏与专辑/歌手同级，只是没有二级栏：直接复用 #library-pane 的单张歌曲表格。
function switchTab(tab) {
  closeTrackContextMenu();
  const prevTab = state.activeTab;
  state.activeTab = tab;
  // #30：modern 皮肤 stage 顶部标题由 CSS 伪元素读取属性；classic 完全不消费。
  document.body.dataset.tab = tab;
  const toolbar = document.querySelector('.toolbar');
  if (toolbar) {
    toolbar.dataset.title = ({
      library: '全部曲库', playlists: '播放列表', albums: '专辑', artists: '歌手',
      favorites: '喜爱', sources: '曲源'
    })[tab] || '全部曲库';
  }
  ['library', 'playlists', 'albums', 'artists', 'favorites', 'sources'].forEach((v) => {
    const btn = $(v === 'favorites' ? 'btn-favorites' : 'tab-' + v);
    if (btn) btn.classList.toggle('active', tab === v);
  });
  // 收藏与曲库共用同一张表格 DOM；其余页面各自独立 pane。
  $('library-pane').hidden = !(tab === 'library' || tab === 'favorites');
  $('playlists-pane').hidden = (tab !== 'playlists');
  $('albums-pane').hidden = (tab !== 'albums');
  $('artists-pane').hidden = (tab !== 'artists');
  $('sources-view-pane').hidden = (tab !== 'sources');
  const fb = $('track-filter-bar');
  if (fb) fb.hidden = (tab === 'favorites');
  const bar = $('favorites-bar');
  if (bar) bar.hidden = (tab !== 'favorites');
  // 按当前页面的布局风格决定：右侧独立详情栏（横向）还是列表上方内联详情条（纵向）。
  applyLayoutStyle();
  // 右侧详情栏是各页面共用的单一 DOM：切页后必须按新页面的选中态重算。
  renderActiveDetail();
  syncSelectionHighlight(tab);
  renderViewDetail(tab);
  // 切到对应面板时载入列表。
  if (tab === 'playlists') {
    loadPlaylists();
    if (state.currentPlaylist) loadPlaylistTracks(state.currentPlaylist.id);
  }
  if (tab === 'albums') loadAlbums();
  if (tab === 'artists') loadArtists();
  // v0.12.7：曲源页复用「二级分组 + 三级曲目」骨架，进入时按归属重新分组。
  if (tab === 'sources') loadSourceGroups();
  // #33：进入收藏视图先同步按当前缓存重绘，再异步取权威集合。不能只等 loadFavorites()，
  // 否则后端稍慢 / 不可用时表格会残留「全部曲库」的行（用户报告「点进去是全部音乐」）。
  if (tab === 'favorites') {
    renderTracks();
    loadFavorites(true);
  }
  // #32：从收藏切回曲库必须重绘表格，否则会停留在收藏集合（空收藏时表现为空白）。
  if (tab === 'library' && prevTab === 'favorites') renderTracks();
  // 删除「刷新」按钮后，切回曲库即重载列表（与切 playlists/albums 等异步 load 同构），
  // 保证列表数据始终为权威；先 renderTracks 防空表闪烁，再 loadTracks 拉取最新。
  if (tab === 'library') loadTracks($('search').value);
}
$('tab-library').addEventListener('click', () => switchTab('library'));
$('tab-playlists').addEventListener('click', () => switchTab('playlists'));
$('tab-albums').addEventListener('click', () => switchTab('albums'));
$('tab-artists').addEventListener('click', () => switchTab('artists'));
$('btn-favorites').addEventListener('click', () => switchTab('favorites'));
$('btn-favorites').addEventListener('dblclick', () => { playAllFavorites(); });
// v0.12.8：第 6 个一级 Tab「曲源」的点击绑定。v0.12.7 只加了 DOM 与
// switchTab('sources') 分支却漏了这一行，真机点击无反应；AA32 用真实点击守护。
$('tab-sources').addEventListener('click', () => switchTab('sources'));

// #30：boot 不调用 switchTab，首屏若已是 modern 皮肤也要有 stage 标题。
(function initStageTitle() {
  document.body.dataset.tab = state.activeTab || 'library';
  const tb = document.querySelector('.toolbar');
  if (tb) tb.dataset.title = '全部曲库';
})();

// ---------- 播放列表（逻辑层已落地：本地曲库内用户自建，local_id 体系）----------

// 载入全部播放列表（含各自曲目数），并渲染左侧列表。
async function loadPlaylists() {
  const { data } = await api('/api/playlists');
  state.playlists = (data && data.playlists) || [];
  renderPlaylists();
}

// ---------- 分组列表公共渲染（播放列表 / 专辑 / 歌手 三页共用） ----------
// 三页的二级列表结构完全同构：<li><span class="pl-name">名称</span><span class="pl-count">曲目数</span></li>
// 仅「唯一键」不同（播放列表用数字 id，专辑/歌手用名称字符串），故用 keyField 参数化。
// 空态统一渲染为「暂无X」提示项（列表本身为空，而非控件无数据）。
function renderGroupList(listId, items, emptyText, keyField, selectedKey) {
  const ul = $(listId);
  if (!ul) return;
  ul.innerHTML = '';
  if (!items || !items.length) {
    ul.innerHTML = '<li class="pl-empty">' + esc(emptyText) + '</li>';
    return;
  }
  items.forEach((it) => {
    const li = document.createElement('li');
    li.dataset[keyField] = it[keyField];
    // 名称为空 → 不渲染名称控件；曲目数为空 → 不渲染数量控件。
    const name = detailValue(it.name);
    const count = detailValue(it.track_count);
    li.innerHTML =
      (name !== null ? `<span class="pl-name">${esc(name)}</span>` : '') +
      (count !== null ? `<span class="pl-count">${esc(count)}</span>` : '');
    if (selectedKey !== null && selectedKey !== undefined && it[keyField] === selectedKey) {
      li.classList.add('selected');
    }
    ul.appendChild(li);
  });
}

// #28：分组的年份 / 流派标签（GroupView 的 genre / year_min / year_max 聚合）。
// 空字段一律返回 null，前端整行跳过，不渲染「—」占位（项目强约束）。
function groupYearLabel(g) {
  if (!g) return null;
  const a = g.year_min, b = g.year_max;
  if (a == null && b == null) return null;
  if (a != null && b != null && a !== b) return `${a}–${b}`;
  return String(a != null ? a : b);
}
function groupTagLine(g) {
  if (!g) return null;
  const parts = [];
  const genre = detailValue(g && g.genre);
  if (genre) parts.push(genre);
  const yr = groupYearLabel(g);
  if (yr) parts.push(yr);
  return parts.length ? parts.join('，') : null;
}

// #28：歌手页二级列表 —— 在既有「名字 + 曲目数」行上追加「流派 · 年份范围」副信息。
// 独立渲染，不改共用 renderGroupList（播放列表 / 专辑列表模式保持原样）。
// li[data-name] / .selected 契约与 renderGroupList 完全一致：点击委托、双击整组播放、
// selectArtist 选中高亮、FR-23 队列来源均不受影响。
function renderArtistList() {
  const ul = $('ar-list');
  if (!ul) return;
  const items = state.artists || [];
  ul.innerHTML = '';
  if (!items.length) {
    ul.innerHTML = '<li class="pl-empty">暂无歌手</li>';
    return;
  }
  const selected = state.currentArtist ? state.currentArtist.name : null;
  items.forEach((it) => {
    const li = document.createElement('li');
    li.dataset.name = it.name;
    const name = detailValue(it.name);
    const count = detailValue(it.track_count);
    const sub = groupTagLine(it);
    li.innerHTML =
      '<span class="pl-name">' + (name !== null ? esc(name) : '') +
        (sub ? `<small class="pl-sub">${esc(sub)}</small>` : '') + '</span>' +
      (count !== null ? `<span class="pl-count">${esc(count)}</span>` : '');
    if (selected !== null && it.name === selected) li.classList.add('selected');
    ul.appendChild(li);
  });
}

// #27：专辑集合「网格视图」——把同一批分组渲染为封面卡片。
// 刻意复用 renderGroupList 的 `li[data-name]` / `.selected` 契约：点击委托、双击整组播放、
// selectAlbum 的选中高亮、FR-23 播放队列来源全部无需改动；只影响二级栏 al-list 的呈现。
// 封面来自 GroupView.cover_id（组内代表曲目，后端 MIN(local_id) 聚合），走既有 /api/cover/:id；
// 无封面时由捕获阶段的 error 委托换成占位符号，不显示「无封面」字样（空字段不渲染）。
function renderGroupCards(listId, items, emptyText, selectedKey) {
  const ul = $(listId);
  if (!ul) return;
  ul.innerHTML = '';
  if (!items || !items.length) {
    ul.innerHTML = '<li class="pl-empty">' + esc(emptyText) + '</li>';
    return;
  }
  items.forEach((it) => {
    const li = document.createElement('li');
    li.dataset.name = it.name;
    const name = detailValue(it.name);
    const count = detailValue(it.track_count);
    // fallback 常驻在封面容器内、被 img 盖住：封面不可用（/api/cover 返回 204/404）时
    // error 委托只把 img 隐藏即可露出占位，不删 img（保留 DOM，便于断言与后续重试）。
    const cover = it.cover_id
      ? `<img src="/api/cover/${Number(it.cover_id)}" alt="" loading="lazy" />`
      : '';
    // #28：副信息 = 年份 · N 首（对齐重设计稿的「歌手 · 年份 · N 首」，
    // 分组模型暂无歌手字段故省略；年份为空时只显示曲目数）。
    const metaParts = [];
    const yr = groupYearLabel(it);
    if (yr) metaParts.push(yr);
    if (count !== null) metaParts.push(`${count} 首`);
    li.innerHTML =
      `<div class="gcard-art">${cover}<span class="gcard-fallback" aria-hidden="true">♫</span></div>` +
      (name !== null ? `<span class="gcard-name">${esc(name)}</span>` : '') +
      (metaParts.length ? `<span class="gcard-count">${esc(metaParts.join('，'))}</span>` : '');
    if (selectedKey !== null && selectedKey !== undefined && it.name === selectedKey) {
      li.classList.add('selected');
    }
    ul.appendChild(li);
  });
}

function renderPlaylists() {
  renderGroupList(
    'pl-list',
    state.playlists,
    '暂无播放列表',
    'id',
    state.currentPlaylist ? state.currentPlaylist.id : null,
  );
}

// 新建播放列表：弹窗输入名称 → 后端创建 → 自动选中并打开。
$('pl-new').addEventListener('click', async () => {
  const raw = await showDialog({
    title: '新建播放列表',
    text: '名称不能为空、不超过 60 个字符，且不与已有列表重名。',
    input: true, okText: '创建',
  });
  if (raw === null) return;
  const name = String(raw).trim();
  if (!name) return;
  setHint('创建播放列表中…');
  const { data } = await api('/api/playlists', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '创建播放列表失败'); return; }
  await loadPlaylists();
  await selectPlaylist(data.id); // 自动选中新建列表
  setHintOk('已创建播放列表：' + name);
});

// 点击左侧列表项 → 选中对应播放列表。
$('pl-list').addEventListener('click', (e) => {
  const li = e.target.closest('li');
  if (!li || li.classList.contains('pl-empty')) return;
  const id = Number(li.dataset.id);
  selectPlaylist(id);
});

// 选中播放列表：高亮、更新标题、显示操作按钮、载入其曲目。
async function selectPlaylist(id) {
  const pl = state.playlists.find((p) => p.id === id);
  state.currentPlaylist = pl || null;
  $('pl-list').querySelectorAll('li').forEach((x) => x.classList.remove('selected'));
  const li = $('pl-list').querySelector('li[data-id="' + id + '"]');
  if (li) li.classList.add('selected');
  $('pl-title').textContent = pl ? pl.name : '播放列表内容';
  $('pl-add-current').hidden = !pl;
  $('pl-add-hint').hidden = !pl;
  $('pl-rename').hidden = !pl;
  $('pl-export').hidden = !pl;
  $('pl-delete').hidden = !pl;
  // 切换播放列表 → 旧列表内的曲级选中不再适用，清空后展示新列表的分组信息。
  state.viewSelection.playlists = null;
  await loadPlaylistTracks(id);
  renderViewDetail('playlists');
  if (state.activeTab === 'playlists') renderActiveDetail();
}

// 载入播放列表曲目并渲染。并发守卫：只认最后一次请求，避免快速切换时旧响应覆盖新列表。
let plReq = 0;
async function loadPlaylistTracks(id) {
  const seq = ++plReq;
  const { data } = await api('/api/playlists/' + id + '/tracks');
  if (seq !== plReq) return;
  state.playlistTracks = (data && data.tracks) || [];
  renderPlaylistTracks();
}

function renderPlaylistTracks() {
  const box = $('pl-content');
  if (!state.currentPlaylist) {
    box.innerHTML =
      '<div class="placeholder">选择播放列表查看内容，或点击「新建」创建。</div>';
    return;
  }
  if (!state.playlistTracks.length) {
    box.innerHTML =
      '<div class="placeholder">该播放列表暂无歌曲。在「全部曲库」选中歌曲后点上方「＋ 加入选中歌曲」，或右键歌曲直接加入。</div>';
    return;
  }
  const cols = visibleTrackCols();
  let html = '<table class="track-table pl-table"><thead><tr>' +
    '<th class="idx-col">#</th>' +
    trackHeadHtml(cols) +
    '<th class="pl-remove-col"></th>' +
    '</tr></thead><tbody>';
  state.playlistTracks.forEach((t, i) => {
    html += `<tr data-id="${t.local_id}">` +
      idxCellHtml(t, i) +
      trackCellsHtml(t, cols) +
      `<td class="pl-remove">` +
        `<button class="md-btn sm" type="button" data-act="up" title="上移">↑</button>` +
        `<button class="md-btn sm" type="button" data-act="down" title="下移">↓</button>` +
        `<button class="md-btn sm" type="button" data-act="remove">移除</button>` +
      `</td>` +
      `</tr>`;
  });
  html += '</tbody></table>';
  box.innerHTML = html;
  setupColumnResize(box.querySelector('.track-table'));
  box.querySelectorAll('tr[data-id]').forEach((tr) => {
    const id = Number(tr.dataset.id);
    const t = state.playlistTracks.find((x) => x.local_id === id);
    tr.addEventListener('click', (e) => {
      if (e.target.closest('button')) return;
      selectTrack(t, tr);
    });
    tr.addEventListener('dblclick', () => { selectTrack(t, tr); playTrackToggleIfCurrent(t); });
    tr.addEventListener('contextmenu', (e) => openTrackContextMenu(e, t, 'playlists', tr));
    tr.querySelector('[data-act="remove"]').addEventListener('click', (e) => {
      e.stopPropagation();
      removeFromPlaylist(id);
    });
    tr.querySelector('[data-act="up"]').addEventListener('click', (e) => {
      e.stopPropagation();
      moveInPlaylist(id, -1);
    });
    tr.querySelector('[data-act="down"]').addEventListener('click', (e) => {
      e.stopPropagation();
      moveInPlaylist(id, 1);
    });
  });
}

// 列表内上移 / 下移（FR-19 一期最小集：按钮重排，拖拽后置）。
async function moveInPlaylist(localId, delta) {
  if (!state.currentPlaylist) return;
  const { data } = await api('/api/playlists/' + state.currentPlaylist.id + '/move', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ local_id: localId, delta: delta }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '调整顺序失败'); return; }
  await loadPlaylistTracks(state.currentPlaylist.id);
  setHint(data.moved ? '顺序已调整' : '已在列表边界');
}

// 重命名（统一弹框，替代原生 prompt 以免抢焦点）。
$('pl-rename').addEventListener('click', async () => {
  if (!state.currentPlaylist) return;
  const raw = await showDialog({
    title: '重命名播放列表',
    text: '名称不能为空、不超过 60 个字符，且不与其它列表重名。',
    input: true, value: state.currentPlaylist.name, okText: '保存',
  });
  if (raw === null) return;
  const name = String(raw).trim();
  if (!name) return;
  const { data } = await api('/api/playlists/' + state.currentPlaylist.id + '/rename', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: name }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '重命名失败'); return; }
  await loadPlaylists();
  await selectPlaylist(state.currentPlaylist.id);
  setHintOk('播放列表已重命名：' + name);
});

// ---------- 导入导出（FR-20）----------
// 导出的是「单个有名列表全量」，绝不导出当前播放队列（否则 n 的截断结果会写进文件）。
$('pl-export').addEventListener('click', async () => {
  if (!state.currentPlaylist) return;
  const pick = await api('/api/pick-folder', { method: 'POST' });
  if (!pick.data || !pick.data.ok || pick.data.canceled || !pick.data.path) return;
  setHint('导出中…');
  const { data } = await api('/api/playlists/' + state.currentPlaylist.id + '/export', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ dir: pick.data.path }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '导出失败'); return; }
  const lines = [
    '列表「' + data.name + '」共 ' + data.track_count + ' 首（全量导出）',
    '已生成：',
  ].concat((data.files || []).map((f) => f));
  lines.push('JSON 为无损备份（可回读），M3U8 为通用交换格式（单向投影）。');
  await showDialog({ title: '导出完成', report: lines, okText: '好' });
  setHintOk('播放列表已导出');
});

// 导入：文件对话框选外部播放列表 → 后端匹配 + 去重 → 新建列表 → 展示完整报告。
$('pl-import').addEventListener('click', async () => {
  const pick = await api('/api/pick-playlist-file', { method: 'POST' });
  if (!pick.data || !pick.data.ok || pick.data.canceled || !pick.data.path) return;
  setHint('导入中…');
  const { data } = await api('/api/playlist-import', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ file: pick.data.path }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '导入失败'); return; }
  const r = data.report || {};
  const lines = [
    '新建列表「' + data.name + '」：外部 ' + data.total + ' 条，写入 ' + data.matched + ' 首。',
  ];
  const groups = [
    ['未匹配（已跳过）', r.unmatched],
    ['重复曲目（保留首次位置）', r.duplicate_ids],
    ['内容级去重（可能误合并不同录音版本）', r.content_duplicates],
    ['匹配歧义（多副本取第一个）', r.ambiguous],
  ];
  groups.forEach(([label, list]) => {
    if (list && list.length) {
      lines.push('— ' + label + '：' + list.length + ' 条');
      list.forEach((x) => lines.push('  ' + x));
    }
  });
  if (lines.length === 1) lines.push('全部条目匹配成功，无重复与未匹配。');
  await showDialog({ title: '导入报告', report: lines, okText: '好' });
  await loadPlaylists();
  await selectPlaylist(data.playlist_id);
  setHintOk('已导入播放列表：' + data.name);
});

// 将「全部曲库」页当前选中的曲目加入当前播放列表。
$('pl-add-current').addEventListener('click', async () => {
  if (!state.currentPlaylist) { setHint('请先选择一个播放列表'); return; }
  // 明确取「全部曲库」页的选中曲（与提示语一致，不受其它页面选中态残留影响）。
  const libSel = state.viewSelection.library;
  if (!libSel || !libSel.track) { setHint('请先在「全部曲库」中选一首歌'); return; }
  const libTrack = libSel.track;
  setHint('加入播放列表中…');
  const { data } = await api('/api/playlists/' + state.currentPlaylist.id + '/add', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ local_id: libTrack.local_id }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '加入失败'); return; }
  setHintOk(`已加入「${state.currentPlaylist.name}」：${libTrack.name}`);
  await loadPlaylists();                      // 刷新左侧曲目计数
  await selectPlaylist(state.currentPlaylist.id); // 重新载入内容并保留高亮
});

// 删除当前播放列表（级联删除条目）。
$('pl-delete').addEventListener('click', async () => {
  if (!state.currentPlaylist) return;
  const yes = await showDialog({
    title: '删除播放列表',
    text: '确定删除播放列表「' + state.currentPlaylist.name + '」？该操作不可撤销。',
    okText: '删除',
  });
  if (!yes) return;
  const id = state.currentPlaylist.id;
  const { data } = await api('/api/playlists/' + id, { method: 'DELETE' });
  if (!data || !data.ok) { setHint((data && data.error) || '删除失败'); return; }
  state.currentPlaylist = null;
  state.viewSelection.playlists = null;
  $('pl-title').textContent = '播放列表内容';
  $('pl-add-current').hidden = true;
  $('pl-add-hint').hidden = true;
  $('pl-rename').hidden = true;
  $('pl-export').hidden = true;
  $('pl-delete').hidden = true;
  await loadPlaylists();
  renderViewDetail('playlists');
  if (state.activeTab === 'playlists') renderActiveDetail();
  $('pl-content').innerHTML =
    '<div class="placeholder">选择播放列表查看内容，或点击「新建」创建。</div>';
  setHintOk('已删除播放列表');
});

// 从播放列表移除曲目。
async function removeFromPlaylist(local_id) {
  if (!state.currentPlaylist) return;
  const { data } = await api('/api/playlists/' + state.currentPlaylist.id + '/remove', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ local_id }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '移除失败'); return; }
  await loadPlaylists();
  await selectPlaylist(state.currentPlaylist.id);
}

// ---------- 专辑 / 歌手（与播放列表同构的三级导航）----------
// 二级列表 = 专辑/歌手分组（含曲目数）；三级 = 选中分组后的歌曲列表。
// 复用 selectTrack / playTrack 选中与播放；复用 .pl-* 样式（与播放列表视觉一致）。

// 通用：把曲目渲染为与播放列表内一致的表格（只读浏览，不含「移除」等编辑操作）。
function renderBrowseTracks(boxId, tracks, emptyMsg) {
  const box = $(boxId);
  // 右键菜单要知道触发来源：al-content = 专辑页，ar-content = 歌手页，sv-content = 曲源页。
  const ctxView = boxId === 'al-content' ? 'albums'
    : (boxId === 'sv-content' ? 'sources' : 'artists');
  if (!tracks.length) {
    box.innerHTML = '<div class="placeholder">' + esc(emptyMsg) + '</div>';
    return;
  }
  const cols = visibleTrackCols();
  let html = '<table class="track-table pl-table"><thead><tr>' +
    '<th class="idx-col">#</th>' +
    trackHeadHtml(cols) +
    '</tr></thead><tbody>';
  tracks.forEach((t, i) => {
    html += '<tr data-id="' + t.local_id + '">' +
      idxCellHtml(t, i) +
      trackCellsHtml(t, cols) +
      '</tr>';
  });
  html += '</tbody></table>';
  box.innerHTML = html;
  setupColumnResize(box.querySelector('.track-table'));
  box.querySelectorAll('tr[data-id]').forEach((tr) => {
    const id = Number(tr.dataset.id);
    const t = tracks.find((x) => x.local_id === id);
    tr.addEventListener('click', (e) => {
      if (e.target.closest('button')) return;
      selectTrack(t, tr);
    });
    tr.addEventListener('dblclick', () => { selectTrack(t, tr); playTrackToggleIfCurrent(t); });
    tr.addEventListener('contextmenu', (e) => openTrackContextMenu(e, t, ctxView, tr));
  });
}

// ---------- 专辑 ----------
async function loadAlbums() {
  const { data } = await api('/api/albums');
  state.albums = (data && data.albums) || [];
  renderAlbums();
}

function renderAlbums() {
  const list = $('al-list');
  const selected = state.currentAlbum ? state.currentAlbum.name : null;
  if (state.albumView === 'grid') {
    if (list) list.classList.add('grid-view');
    renderGroupCards('al-list', state.albums, '暂无专辑', selected);
  } else {
    // 默认「列表视图」= 原有行为，renderGroupList 一字不改，保证既有回归断言全绿。
    if (list) list.classList.remove('grid-view');
    renderGroupList('al-list', state.albums, '暂无专辑', 'name', selected);
  }
}

// #27：切换专辑集合呈现形态。会话内状态，不写 [ui] 配置、不新增定时器。
function setAlbumView(mode) {
  state.albumView = mode === 'grid' ? 'grid' : 'list';
  const seg = $('al-view-seg');
  if (seg) {
    seg.querySelectorAll('button[data-view]').forEach((b) => {
      const on = b.dataset.view === state.albumView;
      b.classList.toggle('on', on);
      b.setAttribute('aria-pressed', String(on));
    });
  }
  renderAlbums();
  // #30：modern 皮肤下 grid 视图升级为「整页封面墙」；classic 一律复位为原三栏。
  syncAlbumWall();
}

// #30：仅 modern + 专辑 grid 生效的封面墙状态。classic 下 album-wall / album-picked
// 全部清除、返回按钮恒隐藏，保证 classic 1:1 不受影响。
function syncAlbumWall() {
  const pane = $('albums-pane');
  const back = $('al-back');
  // 用 <html> 的皮肤类判断，避免 boot 顺序上的变量初始化依赖（TDZ）。
  const wall = document.documentElement.classList.contains('skin-modern') && state.albumView === 'grid';
  if (pane) {
    pane.classList.toggle('album-wall', wall);
    if (!wall) pane.classList.remove('album-picked');
  }
  if (back) back.hidden = !wall;
}

// 视图切换按钮 + 封面兜底（error 不冒泡，须用捕获阶段委托）。
(function initAlbumViewSwitch() {
  const seg = $('al-view-seg');
  if (seg) {
    seg.addEventListener('click', (e) => {
      const btn = e.target.closest('button[data-view]');
      if (btn) setAlbumView(btn.dataset.view);
    });
  }
  document.addEventListener('error', (e) => {
    const img = e.target;
    if (!img || img.tagName !== 'IMG') return;
    // 专辑卡片（#27）与全屏正在播放封面（#29）共用：两者都有常驻 fallback。
    if (!img.closest('.gcard-art') && !img.closest('#nv-cover')) return;
    // 只隐藏 img，露出常驻 fallback；不删节点，避免破图与 DOM 结构抖动。
    img.style.display = 'none';
  }, true);
  // #30：整页封面墙的「← 返回」/ ESC。仅当 album-picked 存在时才动作，classic 恒无副作用。
  const back = $('al-back');
  if (back) {
    back.addEventListener('click', () => {
      const pane = $('albums-pane');
      if (pane) pane.classList.remove('album-picked');
    });
  }
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    const pane = $('albums-pane');
    if (pane && pane.classList.contains('album-picked')) {
      pane.classList.remove('album-picked');
      e.stopPropagation();
    }
  });
})();

$('al-list').addEventListener('click', (e) => {
  const li = e.target.closest('li');
  if (!li || li.classList.contains('pl-empty')) return;
  selectAlbum(li.dataset.name);
});

async function selectAlbum(name) {
  const a = state.albums.find((x) => x.name === name);
  state.currentAlbum = a || null;
  const lis = $('al-list').querySelectorAll('li');
  lis.forEach((x) => x.classList.remove('selected'));
  for (const x of lis) {
    if (x.dataset.name === name) { x.classList.add('selected'); break; }
  }
  $('al-title').textContent = a ? a.name : '专辑内容';
  // #30：modern 封面墙下点卡片 → 浮起专辑曲目表格（← 返回 可回到墙）。
  if (document.documentElement.classList.contains('skin-modern') && state.albumView === 'grid') {
    const pane = $('albums-pane');
    if (pane) pane.classList.add('album-picked');
  }
  // 切换专辑 → 清空该页曲级选中（旧的曲目选中属于上一个专辑）。
  state.viewSelection.albums = null;
  await loadAlbumTracks(name);
  renderViewDetail('albums');
  if (state.activeTab === 'albums') renderActiveDetail();
}

// 并发守卫：只认最后一次请求，避免快速切换专辑时旧响应覆盖新列表。
let albumReq = 0;
async function loadAlbumTracks(name) {
  const seq = ++albumReq;
  const { data } = await api('/api/albums/tracks?name=' + encodeURIComponent(name));
  if (seq !== albumReq) return;
  state.albumTracks = (data && data.tracks) || [];
  renderBrowseTracks('al-content', state.albumTracks, '暂无歌曲。');
}

// ---------- 歌手 ----------
async function loadArtists() {
  const { data } = await api('/api/artists');
  state.artists = (data && data.artists) || [];
  renderArtists();
}

function renderArtists() {
  // #28：歌手页展示流派 / 年份副信息；其余分组仍走共用 renderGroupList。
  renderArtistList();
}

$('ar-list').addEventListener('click', (e) => {
  const li = e.target.closest('li');
  if (!li || li.classList.contains('pl-empty')) return;
  selectArtist(li.dataset.name);
});

async function selectArtist(name) {
  const a = state.artists.find((x) => x.name === name);
  state.currentArtist = a || null;
  const lis = $('ar-list').querySelectorAll('li');
  lis.forEach((x) => x.classList.remove('selected'));
  for (const x of lis) {
    if (x.dataset.name === name) { x.classList.add('selected'); break; }
  }
  $('ar-title').textContent = a ? a.name : '歌手内容';
  // 切换歌手 → 清空该页曲级选中（旧的曲目选中属于上一个歌手）。
  state.viewSelection.artists = null;
  await loadArtistTracks(name);
  renderViewDetail('artists');
  if (state.activeTab === 'artists') renderActiveDetail();
}

// 并发守卫：只认最后一次请求，避免快速切换歌手时旧响应覆盖新列表。
let artistReq = 0;
async function loadArtistTracks(name) {
  const seq = ++artistReq;
  const { data } = await api('/api/artists/tracks?name=' + encodeURIComponent(name));
  if (seq !== artistReq) return;
  state.artistTracks = (data && data.tracks) || [];
  renderBrowseTracks('ar-content', state.artistTracks, '暂无歌曲。');
}

$('btn-settings').addEventListener('click', openSettings);
$('settings-close').addEventListener('click', () => { $('settings-modal').hidden = true; });
// 打开「更改 / 切换曲库」对话框（设置内按钮与顶栏 ✎ 共用同一流程）。
function openChangeLibrary() {
  $('settings-modal').hidden = true;
  $('library-dir-modal').hidden = false;
  $('library-dir-input').value = state.library_dir;
  $('library-dir-input').focus();
}
$('set-change-library').addEventListener('click', openChangeLibrary);
// 顶栏 ✎：直接切换曲库（点击即可选择其它曲库目录）。
$('btn-switch-library').addEventListener('click', openChangeLibrary);
// 顶栏曲库名称也可点击触发切换。
$('library-name').addEventListener('click', openChangeLibrary);
// 修改曲库名称：写回本曲库 .mt/config.toml（多曲库 / 多用户区分）。
$('set-name-apply').addEventListener('click', async () => {
  const name = $('set-name-input').value.trim();
  if (!name) { setHint('请输入曲库名称'); return; }
  setHint('保存曲库名称中…');
  const { data } = await api('/api/library-name', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '保存失败'); return; }
  state.library_name = data.library_name || name;
  setHintOk('曲库名称已保存：' + state.library_name);
  updateLibraryName();
  await refreshStatus();
});
$('set-apply-scheme').addEventListener('click', applyScheme);

// ---------- 设置面板：左侧标签导航 ↔ 右侧分栏内容 ----------
// 纯前端 class 切换，不改动任何设置项的控件与业务逻辑。
function activateSettingsTab(key) {
  const tabs = document.querySelectorAll('#settings-modal .settings-nav > .settings-tab');
  const panes = document.querySelectorAll('#settings-modal .settings-content > .settings-pane');
  const valid = Array.from(tabs).some((t) => t.dataset.tab === key);
  const target = valid ? key : 'appearance';
  tabs.forEach((t) => {
    const on = t.dataset.tab === target;
    t.classList.toggle('active', on);
    t.setAttribute('aria-selected', on ? 'true' : 'false');
  });
  panes.forEach((p) => p.classList.toggle('active', p.dataset.pane === target));
  const content = document.querySelector('#settings-modal .settings-content');
  if (content) content.scrollTop = 0;   // 切标签后右侧回到顶部
}
const settingsNav = document.querySelector('#settings-modal .settings-nav');
if (settingsNav) settingsNav.setAttribute('role', 'tablist');
document.querySelectorAll('#settings-modal .settings-nav > .settings-tab').forEach((tab) => {
  tab.setAttribute('role', 'tab');
  tab.addEventListener('click', () => {
    // v0.12.11：设置里「曲源总览」是普通子页面，不弹独立窗口；切进去时刷新四块常显内容。
    if (tab.dataset.tab === 'import') { activateSettingsTab('import'); refreshSourcesPage(); return; }
    if (tab.dataset.tab === 'plugins') { activateSettingsTab('plugins'); refreshPluginsPage(); return; }
    activateSettingsTab(tab.dataset.tab);
  });
});
document.querySelectorAll('#settings-modal .settings-pane').forEach((pane) => {
  pane.setAttribute('role', 'tabpanel');
});

// 文件结构方案：卡片选中态（:has() 为主，.is-checked 兼容旧 WebView）
function syncSchemeOptionState() {
  document.querySelectorAll('#set-scheme-options label').forEach((label) => {
    const input = label.querySelector('input[type="radio"]');
    if (input) label.classList.toggle('is-checked', input.checked);
  });
}
document.querySelectorAll('#set-scheme-options input').forEach((r) =>
  r.addEventListener('change', syncSchemeOptionState)
);

// 设置里预选当前方案
function openSettings() {
  $('settings-modal').hidden = false;
  $('set-library-path').textContent = state.library_dir || '（未设置）';
  $('set-name-input').value = state.library_name || '';
  document.querySelectorAll('#set-scheme-options input').forEach((r) => {
    r.checked = (r.value === state.file_scheme);
  });
  syncSchemeOptionState();
  renderColConfig();
  renderLayoutStyleUI();
  renderColWidthUI();
  renderAppearanceUI();
  renderAbout();
  renderConfigStatus();
  $('set-auto-count').value = String(state.autoPlaylistCount);
  $('set-auto-count-hint').textContent = (typeof window.KQK.t === 'function')
    ? window.KQK.t('settings.playback.auto_count', {
        count: state.autoPlaylistCount,
        _fallback: '当前：' + state.autoPlaylistCount + ' 首'
      })
    : ('当前：' + state.autoPlaylistCount + ' 首');
}

// ---------- FR-25：歌曲列表表头右键 → 设置 → 列表 → 歌曲列表显示项 ----------
// 四张歌曲表的表头都由 JS 动态重绘（innerHTML），因此用 document 级委托：
// 重绘后无需重新绑定，表头文字与列宽拖拽手柄上的右键都能命中。
let colSectionFlashTimer = 0;
function openSettingsColumns() {
  openSettings();
  activateSettingsTab('columns');
  const content = document.querySelector('#settings-modal .settings-content');
  const section = $('set-columns-section');
  if (!content || !section) return;
  const delta = section.getBoundingClientRect().top - content.getBoundingClientRect().top;
  content.scrollTop = Math.max(0, content.scrollTop + delta - 8);
  section.classList.remove('focus-flash');
  void section.offsetWidth;   // 强制重排，连续右键时动画也能重播。
  section.classList.add('focus-flash');
  clearTimeout(colSectionFlashTimer);
  colSectionFlashTimer = setTimeout(() => section.classList.remove('focus-flash'), 1800);
}
document.addEventListener('contextmenu', (e) => {
  const el = e.target;
  const th = (el && el.closest) ? el.closest('.track-table thead th') : null;
  if (!th) return;
  e.preventDefault();          // 不弹浏览器原生菜单
  closeTrackContextMenu();     // 表头右键不属于曲目右键菜单
  openSettingsColumns();
});

// 渲染「歌曲列表显示项」配置（勾选可见 + ↑/↓ 调整顺序）。
function renderColConfig() {
  const ul = $('col-config-list');
  ul.innerHTML = '';
  state.columns.forEach((key, idx) => {
    const def = COLUMN_DEFS[key];
    const li = document.createElement('li');
    li.dataset.key = key;
    li.innerHTML =
      `<input class="col-vis" type="checkbox" ${state.visible.has(key) ? 'checked' : ''} data-act="vis" />` +
      `<span class="col-label">${esc(def.label)}</span>` +
      `<button class="md-btn col-move" data-act="up" ${idx === 0 ? 'disabled' : ''}>↑</button>` +
      `<button class="md-btn col-move" data-act="down" ${idx === state.columns.length - 1 ? 'disabled' : ''}>↓</button>`;
    ul.appendChild(li);
  });
}

// 渲染「布局风格」配置：每个页面一行 + 横向 / 纵向 分段控件。
function renderLayoutStyleUI() {
  const box = $('layout-style-list');
  if (!box) return;
  box.innerHTML = '';
  VIEW_KEYS.forEach((v) => {
    const cur = state.layoutStyles[v] || 1;
    const row = document.createElement('div');
    row.className = 'layout-style-row';
    row.innerHTML =
      `<span class="layout-style-label">${esc(LAYOUT_VIEWS[v].label)}</span>` +
      '<div class="seg">' +
      `<button class="seg-btn${cur === 1 ? ' active' : ''}" type="button" data-view="${v}" data-style="1">${LAYOUT_STYLE_LABELS[1]}</button>` +
      `<button class="seg-btn${cur === 2 ? ' active' : ''}" type="button" data-view="${v}" data-style="2">${LAYOUT_STYLE_LABELS[2]}</button>` +
      '</div>';
    box.appendChild(row);
  });
}

// 切换某页面的布局风格：立即生效 + 持久化。
$('layout-style-list').addEventListener('click', async (e) => {
  const btn = e.target.closest('.seg-btn');
  if (!btn) return;
  const view = btn.dataset.view;
  const style = Number(btn.dataset.style);
  if (!LAYOUT_VIEWS[view] || (style !== 1 && style !== 2)) return;
  state.layoutStyles[view] = style;
  renderLayoutStyleUI();
  applyLayoutStyle();
  // 当前页若已渲染歌曲列表，按新宽度重新分配列宽。
  scheduleColumnWidths();
  const { data } = await api('/api/ui-prefs', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ layout_styles: { [view]: style } }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '保存布局风格失败'); return; }
  setHintOk(LAYOUT_VIEWS[view].label + ' 已切换为' + LAYOUT_STYLE_LABELS[style]);
});

// 渲染「列宽比例」列表（各列当前占比 + 占比条）。
function renderColWidthUI() {
  const ul = $('col-width-list');
  if (!ul) return;
  ul.innerHTML = '';
  state.columns.forEach((key) => {
    const def = COLUMN_DEFS[key];
    const pct = Number(state.columnWidths[key]) || 0;
    const li = document.createElement('li');
    li.dataset.key = key;
    li.innerHTML =
      `<span class="col-label">${esc(def.label)}</span>` +
      `<span class="col-bar"><i style="width:${Math.max(0, Math.min(100, pct))}%"></i></span>` +
      `<span class="col-pct">${pct.toFixed(1)}%</span>`;
    ul.appendChild(li);
  });
}

// 恢复默认列宽比例。
$('set-reset-widths').addEventListener('click', async () => {
  state.columnWidths = Object.assign({}, DEFAULT_COL_WIDTHS);
  renderColWidthUI();
  scheduleColumnWidths();
  await saveColumnWidths();
  setHintOk('已恢复默认列宽比例');
});

// 列配置交互：勾选切换可见、↑/↓ 调整顺序。
$('col-config-list').addEventListener('click', (e) => {
  const btn = e.target.closest('button[data-act]');
  if (!btn) return;
  const li = btn.closest('li');
  const key = li.dataset.key;
  const idx = state.columns.indexOf(key);
  if (btn.dataset.act === 'up' && idx > 0) {
    [state.columns[idx - 1], state.columns[idx]] = [state.columns[idx], state.columns[idx - 1]];
  } else if (btn.dataset.act === 'down' && idx < state.columns.length - 1) {
    [state.columns[idx + 1], state.columns[idx]] = [state.columns[idx], state.columns[idx + 1]];
  }
  renderColConfig();
});
$('col-config-list').addEventListener('change', (e) => {
  const cb = e.target.closest('input[data-act="vis"]');
  if (!cb) return;
  const key = e.target.closest('li').dataset.key;
  if (cb.checked) state.visible.add(key);
  else state.visible.delete(key);
});
$('set-apply-columns').addEventListener('click', async () => {
  const cols = state.columns.filter((k) => state.visible.has(k));
  if (!cols.length) { setHint('至少保留一列'); return; }
  const { data } = await api('/api/columns', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ columns: cols }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '保存显示设置失败'); return; }
  state.columns = data.columns.concat(ALL_COLUMN_KEYS.filter((k) => !new Set(data.columns).has(k)));
  state.visible = new Set(data.columns);
  renderColConfig();
  renderTracks();
  setHintOk('歌曲列表显示设置已保存');
});

// 设置 → 播放：n 写 app 级 [playback] 段，下次生成队列才生效（当前队列的 n 已快照）。
$('set-apply-playback').addEventListener('click', async () => {
  const n = clampAutoCount($('set-auto-count').value);
  $('set-auto-count').value = String(n);
  const { data } = await api('/api/playback-prefs', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ auto_playlist_count: n }),
  });
  if (!data || !data.ok) {
    $('set-auto-count-hint').textContent = (data && data.error) || '保存失败';
    setHint('自动播放列表上限保存失败');
    return;
  }
  state.autoPlaylistCount = clampAutoCount(data.auto_playlist_count);
  $('set-auto-count').value = String(state.autoPlaylistCount);
  $('set-auto-count-hint').textContent =
    '已保存：' + state.autoPlaylistCount + ' 首（下次播放生效）';
  setHintOk('自动播放列表上限已保存：' + state.autoPlaylistCount + ' 首');
});

$('library-dir-cancel').addEventListener('click', () => {
  $('library-dir-modal').hidden = true;
});
$('library-dir-browse').addEventListener('click', browseLibraryDirectory);
$('library-dir-save').addEventListener('click', saveLibraryDirectory);

async function browseLibraryDirectory() {
  const button = $('library-dir-browse');
  button.disabled = true;
  const { data } = await api('/api/pick-library-directory', { method: 'POST' });
  button.disabled = false;
  if (data && data.ok && !data.canceled && data.path) {
    $('library-dir-input').value = data.path;
  } else if (!data || !data.ok) {
    setHint((data && data.error) || '打开目录选择窗口失败');
  }
}

async function saveLibraryDirectory() {
  const button = $('library-dir-save');
  button.disabled = true;
  setHint('保存曲库目录中…');
  const { data } = await api('/api/library-directory', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path: $('library-dir-input').value }),
  });
  button.disabled = false;
  if (!data || !data.ok) {
    setHint((data && data.error) || '保存曲库目录失败');
    return;
  }

  state.library_dir = data.library_dir;
  $('library-dir-modal').hidden = true;
  setHint('曲库目录已设置，正在扫描…');
  const scan = await api('/api/scan', { method: 'POST' });
  if (scan.data && scan.data.ok) {
    setHintOk(`曲库目录已更新：${state.library_dir}（新增 ${scan.data.inserted}，更新 ${scan.data.updated}，移除 ${scan.data.removed}）`);
    maybeShowNcmOrganize(scan.data.ncm_found || [], true);
    if ((scan.data.pending_duplicates || 0) > 0) await openDuplicatesDialog();
  } else {
    setHint(`目录已保存，但扫描失败：${(scan.data && scan.data.error) || '请稍后重试'}`);
  }
  await refreshStatus();
}

// ---------- 导入文件夹（通用：复制/移动到曲库）----------
// 卡片式单选选中态：:has() 为主，.is-checked 兼容旧版 WebView（切换时同步）。
function syncImportOptionState() {
  document.querySelectorAll('#import-modal .imp-option').forEach((label) => {
    const input = label.querySelector('input[type="radio"]');
    if (input) label.classList.toggle('is-checked', input.checked);
  });
}
document.querySelectorAll('#import-modal input[name="import-mode"]').forEach((r) =>
  r.addEventListener('change', syncImportOptionState)
);
syncImportOptionState();
const closeImportModal = () => { $('import-modal').hidden = true; };
$('import-cancel').addEventListener('click', closeImportModal);
$('import-close').addEventListener('click', closeImportModal);
$('import-browse').addEventListener('click', async () => {
  const { data } = await api('/api/pick-folder', { method: 'POST' });
  if (data && data.ok && !data.canceled && data.path) {
    $('import-input').value = data.path;
  } else if (!data || !data.ok) {
    setHint((data && data.error) || '打开文件夹选择失败');
  }
});
$('import-run').addEventListener('click', async () => {
  const path = $('import-input').value.trim();
  if (!path) { setHint('请先选择要导入的文件夹'); return; }
  const mode = document.querySelector('input[name="import-mode"]:checked').value;
  $('import-modal').hidden = true;
  setHint('导入中…');
  // 打开进度面板并开始轮询，显示逐首明细。
  showProgressPanel(mode === 'move' ? '移动导入' : '复制导入');
  startProgressPolling();
  const { data } = await api('/api/import', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path, mode }),
  });
  finishProgressTask();
  setHint(data && data.ok
    ? `导入完成：新增 ${data.imported}，跳过 ${data.skipped}${data.ncm_skipped ? `（含 ${data.ncm_skipped} 个加密文件，未配置工具）` : ''}${data.held ? `，重复挂起 ${data.held}` : ''}，失败 ${data.failed}`
    : (data && data.error) || '导入失败');
  // FR-22 P4：批量结束后一起弹对比选择窗（全部未决组一次展示）。
  await refreshDuplicateBadge();
  if (data && data.ok && data.held) await openDuplicatesDialog();
  loadTracks($('search').value);
});

// ==========================================================================
// v0.12.11 曲源总览：设置子页（四块单栏：按钮 / 列表+详情 / 待处理重复 / NCM 工具）
// + 「添加曲源」独立窗口（另建一套 add-src-* 表单，与右侧 src-* 详情互不干扰）。
// 事实源 = app 级 [[sources]]（/api/sources）；旧端点 /api/import-netease、/api/netease/library-dir、
// /api/import 与 CLI 全部保留不变。v0.12.11 起「检查 / 一键导入 / 持续监听」只读检查链下线（后端端点保留）。
// ==========================================================================
state.sources = [];
state.sourcesCurrentId = '';
// v0.12.12：内置特殊曲源「本地曲库」的 id，与后端 db::UNSPECIFIED_SOURCE / groups 的
// unspecified_id 一致。它只存在于前端列表（绝不写 [[sources]] 配置），因此天然不参与
// 曲源 CRUD 与「更新全部曲源」；曲库文件夹内由扫描登记的歌曲归到这个分组。
const LOCAL_LIBRARY_ID = 'local_scan';

/** 打开设置并落到「曲源总览」子页；四块常显内容一次性刷新，无新增定时器。 */
async function openSourcesPage() {
  openSettings();
  activateSettingsTab('import');
  await refreshSourcesPage();
}

/** 刷新子页四块数据：来源列表 + 待处理重复；NCM 状态随 loadSources 一并回填。 */
async function refreshSourcesPage() {
  await loadSources();
  await loadSourcesDuplicates();
  const tip = $('ov-tip');
  if (tip) tip.dataset.tip = sourceLabels().tipReadonly;
  const nameTip = $('src-name-tip');
  if (nameTip) nameTip.dataset.tip = sourceLabels().tipRename;
  renderSourceOverviewStats();
  syncSrcBlockLayout();
}

/** v0.16.11 曲源总览·歌曲数：取 /api/sources/groups 的 count 求和（含内置曲库扫描入库的曲目）。
 *  端点已存在，无新增路由；曲源数 / 启用数在 renderSourcesList() 内同步更新。 */
async function renderSourceOverviewStats() {
  const { data } = await api('/api/sources/groups');
  const groups = (data && data.ok && Array.isArray(data.groups)) ? data.groups : [];
  const total = groups.reduce((n, g) => n + (Number(g.count) || 0), 0);
  const el = $('ov-stat-tracks');
  if (el) el.textContent = String(total);
}

/** v0.14.9：四块左右分栏仲裁。按钮始终在右栏（网格两栏固定），右栏内默认横排；
 *  仅当左栏自然高 >= 按钮竖排高时加 .side-actions 改竖排。收起态（右栏隐藏）跳过。resize 防抖重测。 */
let srcLayoutTimer = null;
function renderImportPrefs() {
  const size = $('set-min-size');
  const dur = $('set-min-duration');
  if (size && dur) {
    size.value = String(state.importPrefs.minSizeKb);
    dur.value = String(state.importPrefs.minDurationSec);
  }
  const auto = $('ov-auto-update-on-start');
  if (auto) auto.checked = !!state.importPrefs.autoUpdateSourcesOnStart;
}

function clampImportPref(raw, max, fallback) {
  const n = Number(raw);
  if (!Number.isFinite(n) || n < 0 || n > max) return fallback;
  return Math.round(n);
}

{
  const apply = $('set-apply-filter');
  if (apply) apply.addEventListener('click', async () => {
    const minSizeKb = clampImportPref($('set-min-size').value, 102400, state.importPrefs.minSizeKb);
    const minDurationSec = clampImportPref($('set-min-duration').value, 3600, state.importPrefs.minDurationSec);
    const { data } = await api('/api/import-prefs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ min_size_kb: minSizeKb, min_duration_sec: minDurationSec }),
    });
    if (!data || !data.ok) {
      setHint((data && data.error) || '保存歌曲筛选设置失败');
      return;
    }
    state.importPrefs.minSizeKb = Number(data.min_size_kb);
    state.importPrefs.minDurationSec = Number(data.min_duration_sec);
    renderImportPrefs();
    setHintOk('歌曲筛选设置已保存');
  });
}

{
  const auto = $('ov-auto-update-on-start');
  if (auto) auto.addEventListener('change', async () => {
    const { data } = await api('/api/import-prefs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ auto_update_sources_on_start: auto.checked }),
    });
    if (!data || !data.ok) {
      setHint((data && data.error) || '保存启动自动更新设置失败');
      auto.checked = !!state.importPrefs.autoUpdateSourcesOnStart;
      return;
    }
    state.importPrefs.autoUpdateSourcesOnStart = !!data.auto_update_sources_on_start;
    auto.checked = state.importPrefs.autoUpdateSourcesOnStart;
    setHintOk(auto.checked ? '已开启启动后自动更新曲源' : '已关闭启动后自动更新曲源');
  });
}

function syncSrcBlockLayout() {
  const pane = document.querySelector('#settings-modal .src-pane[data-pane="import"]');
  if (!pane || !pane.classList.contains('active') || pane.hidden) return;
  pane.querySelectorAll('.src-pane-block').forEach((block) => {
    const main = block.querySelector('.src-block-main');
    const actions = block.querySelector('.src-block-actions');
    if (!main || !actions) return;
    // v0.14.12：曲源信息块（列表+详情）空间恒定足够，按钮列固定竖排，不参与动态仲裁。
    if (block.id === 'src-info-block') { block.classList.add('side-actions'); return; }
    if (block.classList.contains('no-selection')) { block.classList.remove('side-actions'); return; }
    // 其余块：横排态读左栏自然高度，再临时竖排读按钮列占用高度，左栏 >= 按钮列才保持竖排。
    block.classList.remove('side-actions');
    const mainH = main.getBoundingClientRect().height;
    block.classList.add('side-actions');
    const actionsH = actions.getBoundingClientRect().height;
    block.classList.toggle('side-actions', mainH >= actionsH);
  });
}
window.addEventListener('resize', () => {
  if (srcLayoutTimer) clearTimeout(srcLayoutTimer);
  srcLayoutTimer = setTimeout(syncSrcBlockLayout, 120);
});

/** 添加曲源独立窗口：重置表单后显示，焦点落在名称输入。 */
function openSourceAddModal() {
  $('add-src-name').value = '';
  $('add-src-path').value = '';
  $('add-src-netease').checked = false;
  $('add-src-enabled').checked = true;
  $('add-src-probe').textContent = '';
  document.querySelectorAll('input[name="add-src-mode"]').forEach((r) => {
    r.checked = r.value === 'copy';
  });
  syncSourceModeUI('add-src');
  $('sources-modal').hidden = false;
  $('add-src-name').focus();
}

function closeSourceAddModal() {
  $('sources-modal').hidden = true;
}

/** 拉取来源列表，顺带刷新角标与 NCM 工具状态（一次请求，无新增轮询）。 */
async function loadSources() {
  const { data } = await api('/api/sources');
  if (!data || !data.ok) {
    setHint((data && data.error) || '读取曲源失败');
    return;
  }
  state.sources = data.sources || [];
  if (typeof data.pending_duplicates === 'number') updateDupBadge(data.pending_duplicates);
  if (typeof data.ncm_ready === 'boolean') state.ncm_ready = data.ncm_ready;
  // 内置「本地曲库」不在 /api/sources 返回的真实列表里，选中态不能被这段清理掉。
  if (state.sourcesCurrentId && state.sourcesCurrentId !== LOCAL_LIBRARY_ID
      && !state.sources.some((s) => s.id === state.sourcesCurrentId)) {
    state.sourcesCurrentId = '';
  }
  renderSourcesList();
  renderSourceForm();
}

/** 曲源信息列表（v0.12.14 A1）：与歌曲表同口径的简约紧凑单行 —— 名称 + 状态圆点。
 *  路径 / 类型 / 导入方式 / 启用 / 状态文案不再常显，统一放进 title 悬停提示；
 *  右侧详情表单仍是完整信息源。class / data-source-id / .src-state / .is-off / .is-builtin 保持不变。 */
function renderSourcesList() {
  const box = $('sources-list');
  if (!box) return;
  const L = sourceLabels();
  box.innerHTML = '';
  if (!state.sources.length) {
    const empty = document.createElement('div');
    empty.className = 'sources-empty';
    empty.textContent = '暂无外部曲源';
    box.appendChild(empty);
  }
  state.sources.forEach((s) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'settings-tab src-item' + (s.enabled ? '' : ' is-off')
      + (s.id === state.sourcesCurrentId ? ' active' : '');
    btn.dataset.sourceId = s.id;
    const name = s.name || s.path || '未命名曲源';
    const path = s.path || '未设置目录';
    const st = state.sourceStatus[s.id] || 'idle';
    const stLabel = { idle: L.stIdle, running: L.stRunning, done: L.stDone, error: L.stError }[st] || L.stIdle;
    btn.title = `${name}\n${path}\n${s.is_netease ? L.typeNetease : L.typeLocal}，`
      + `${s.mode === 'move' ? L.modeMove : L.modeCopy}，${s.enabled ? L.enabled : L.disabled}，${stLabel}`;
    btn.innerHTML = `<span class="src-row"><strong>${esc(name)}</strong>` +
      `<span class="src-state st-${esc(st)}" title="${esc(stLabel)}"></span></span>`;
    btn.addEventListener('click', () => {
      // v0.14.9：点已展开的曲源再点一次 = 收起（chevron ▾→▸）；点别的 = 展开该项。
      state.sourcesCurrentId = state.sourcesCurrentId === s.id ? '' : s.id;
      renderSourcesList();
      renderSourceForm();
    });
    box.appendChild(btn);
  });
  // v0.12.12：内置特殊曲源，永远排在最后（与后端 groups 的 chain(once(...)) 顺序一致）。
  // 不写配置、不占 id、不参与更新；点它只展示只读详情，删除 / 改名 / 导入入口全部不可用。
  const builtin = document.createElement('button');
  builtin.type = 'button';
  builtin.className = 'settings-tab src-item is-builtin'
    + (state.sourcesCurrentId === LOCAL_LIBRARY_ID ? ' active' : '');
  builtin.dataset.sourceId = LOCAL_LIBRARY_ID;
  builtin.dataset.builtin = '1';
  builtin.title = `${L.localLibrary}\n${state.library_dir || '（未设置曲库目录）'}\n`
    + `${L.typeLibrary}，${L.builtinMeta}`;
  builtin.innerHTML = `<span class="src-row"><strong>${esc(L.localLibrary)}</strong>` +
    `<span class="src-state st-builtin" title="${esc(L.typeLibrary)}，${esc(L.builtinMeta)}"></span></span>`;
  builtin.addEventListener('click', () => {
    state.sourcesCurrentId = state.sourcesCurrentId === LOCAL_LIBRARY_ID ? '' : LOCAL_LIBRARY_ID;
    renderSourcesList();
    renderSourceForm();
  });
  box.appendChild(builtin);
  // v0.16.11：曲源数 / 启用数随列表实时更新（内置 local_scan 不计入）。
  const src = state.sources || [];
  const cSrc = $('ov-stat-sources'), cOn = $('ov-stat-enabled');
  if (cSrc) cSrc.textContent = String(src.length);
  if (cOn) cOn.textContent = String(src.filter((s) => s.enabled).length);
}

// ==========================================================================
// v0.12.11 曲源列表概要 / 串行更新汇总
// 所有动态文案集中在此处以字面量 key 调用翻译函数，供 check_i18n.py 校验无未使用条目。
// ==========================================================================
state.sourceUpdatingAll = false;
state.sourceSummary = { done: 0, total: 0, imported: 0, skipped: 0, filtered: 0, intra: 0, upgraded: 0, low: 0 };
state.unspecifiedSourceId = LOCAL_LIBRARY_ID;

function sourceLabels() {
  return {
    importOnce: t('sources.import_once', { _fallback: '更新' }),
    del: t('sources.delete', { _fallback: '删除' }),
    enabled: t('sources.enabled', { _fallback: '启用' }),
    disabled: t('sources.disabled', { _fallback: '停用' }),
    stIdle: t('sources.status_idle', { _fallback: '待导入' }),
    stRunning: t('sources.status_running', { _fallback: '导入中' }),
    stDone: t('sources.status_done', { _fallback: '已完成' }),
    stError: t('sources.status_error', { _fallback: '导入失败' }),
    modeCopy: t('sources.mode_copy', { _fallback: '复制' }),
    modeMove: t('sources.mode_move', { _fallback: '移动' }),
    typeNetease: t('sources.type_netease', { _fallback: '网易云曲源' }),
    typeLocal: t('sources.type_local', { _fallback: '本地曲源' }),
    localLibrary: t('sources.local_library', { _fallback: '本地曲库' }),
    typeLibrary: t('sources.type_library', { _fallback: '内置曲源' }),
    builtinMeta: t('sources.builtin_meta', { _fallback: '内置，不可删除' }),
    builtinTip: t('sources.builtin_tip', {
      _fallback: '曲库文件夹内由扫描登记的歌曲自动归入此曲源。它是内置项，不能改名、不能删除，也不参与「导入全部曲源」。' }),
    tipReadonly: t('sources.tip_readonly', {
      _fallback: '曲库是本地管理的曲库文件夹，曲源是外部上游目录；这里的重命名与删除不会改动曲库，也不会改动原始文件夹。' }),
    tipRename: t('sources.tip_rename', {
      _fallback: '仅改显示名称，不影响实际目录，也不改变已导入歌曲的归属。' }),
  };
}

function sourceGroupName(g) {
  if (!g) return '';
  return g.id === state.unspecifiedSourceId
    ? t('sources.local_library', { _fallback: '本地曲库' })
    : (g.name || g.id);
}

let sourceSummaryHideTimer = null;

// v0.16.2：汇总行仅作更新/导入后的操作反馈；空闲态隐藏。
// settle=true 表示本次操作已收尾，2.5s 后自动收起（与进度面板完成态节奏一致）。
function renderSourceSummary(settle) {
  const s = state.sourceSummary;
  const el = $('ov-summary');
  if (!el) return;
  el.textContent = t('sources.summary', {
    done: s.done, total: s.total, imported: s.imported, skipped: s.skipped,
    filtered: s.filtered || 0, intra: s.intra || 0,
    upgraded: s.upgraded || 0, low: s.low || 0,
    _fallback: `已完成 ${s.done}/${s.total}，入库 ${s.imported}，跳过 ${s.skipped}，筛除 ${s.filtered || 0}，同源跳过 ${s.intra || 0}，升级 ${s.upgraded || 0}，低质忽略 ${s.low || 0}`,
  });
  el.hidden = false;
  if (sourceSummaryHideTimer) { clearTimeout(sourceSummaryHideTimer); sourceSummaryHideTimer = null; }
  if (settle) {
    sourceSummaryHideTimer = setTimeout(() => {
      el.hidden = true;
      sourceSummaryHideTimer = null;
    }, 2500);
  }
}

/** 更新全部曲源：严格按列表顺序串行 await（全局只有一份 import_progress，并发会互相覆盖）。 */
async function updateAllEnabledSources() {
  if (state.sourceUpdatingAll) return;
  const enabled = (state.sources || []).filter((s) => s.enabled);
  state.sourceUpdatingAll = true;
  state.sourceSummary = { done: 0, total: enabled.length, imported: 0, skipped: 0, filtered: 0, intra: 0, upgraded: 0, low: 0 };
  renderSourceSummary();
  const btn = $('ov-update-all');
  if (btn) btn.disabled = true;
  try {
    for (const s of enabled) {
      const r = await importSource(s.id, { silent: true });
      state.sourceSummary.done += 1;
      if (r && r.ok) {
        state.sourceSummary.imported += r.imported || 0;
        state.sourceSummary.skipped += r.skipped || 0;
        state.sourceSummary.filtered += r.filtered || 0;
        state.sourceSummary.intra += r.intra_source_skipped || 0;
        state.sourceSummary.upgraded += r.upgraded || 0;
        state.sourceSummary.low += r.lower_quality_ignored || 0;
      }
      renderSourceSummary();
    }
  } finally {
    state.sourceUpdatingAll = false;
    if (btn) btn.disabled = false;
    renderSourceSummary(true);
    if (state.activeTab === 'sources') await loadSourceGroups();
  }
}

// ==========================================================================
// v0.12.7 rail 曲源页：二级按 source.id 分组，三级复用 renderBrowseTracks()
// ==========================================================================
async function loadSourceGroups() {
  const { data } = await api('/api/sources/groups');
  if (!data || !data.ok) return;
  state.sourceGroups = data.groups || [];
  state.unspecifiedSourceId = data.unspecified_id || 'local_scan';
  const cur = state.currentSource && state.currentSource.id;
  if (cur && !state.sourceGroups.some((g) => g.id === cur)) state.currentSource = null;
  renderSourceGroups();
  if (state.currentSource) await loadSourceTracks(state.currentSource.id);
}

function renderSourceGroups() {
  const items = (state.sourceGroups || []).map((g) => ({
    id: g.id, name: sourceGroupName(g), track_count: g.count || 0,
  }));
  renderGroupList('sv-list', items, '暂无曲源', 'id', state.currentSource && state.currentSource.id);
}

async function loadSourceTracks(id) {
  const { data } = await api('/api/sources/tracks?id=' + encodeURIComponent(id));
  state.currentSource = (state.sourceGroups || []).find((g) => g.id === id)
    || { id: id, name: id, count: 0 };
  state.sourceTracks = (data && data.tracks) || [];
  const title = $('sv-title');
  if (title) title.textContent = sourceGroupName(state.currentSource);
  const cnt = $('sv-count');
  if (cnt) cnt.textContent = state.sourceTracks.length + ' 首';
  renderSourceGroups();
  renderBrowseTracks('sv-content', state.sourceTracks, '该曲源暂无歌曲');
  state.viewSelection.sources = null;
  renderActiveDetail();
}


/** 内置「本地曲库」只读态：禁用字段 + 禁用按钮（CSS 按 .is-builtin 另把右栏动作整体隐藏）。 */
function setSourceFormReadonly(ro) {
  $('src-form').classList.toggle('is-builtin', ro);
  const block = $('src-info-block');
  if (block) block.classList.toggle('is-builtin', ro);
  ['src-name', 'src-path', 'src-netease', 'src-enabled', 'src-browse',
    'src-save', 'src-import', 'src-delete'].forEach((id) => {
    const el = $(id);
    if (el) el.disabled = ro;
  });
  document.querySelectorAll('input[name="src-mode"]').forEach((r) => { r.disabled = ro; });
}

/** 右侧详情：真实曲源可编辑；内置「本地曲库」只展示只读信息，不提供任何写操作。 */
function renderSourceForm() {
  const builtin = state.sourcesCurrentId === LOCAL_LIBRARY_ID;
  const s = builtin ? null : state.sources.find((x) => x.id === state.sourcesCurrentId);
  const L = sourceLabels();
  $('src-form').hidden = !s && !builtin;
  $('src-empty').hidden = !!s || builtin;
  // v0.14.9：收起（未选中任何曲源）时右栏动作一并隐藏，避免留下无目标的按钮。
  const infoBlock = $('src-info-block');
  if (infoBlock) infoBlock.classList.toggle('no-selection', !s && !builtin);
  // 每次都先复位，保证从内置项切回真实曲源时表单完全恢复可编辑。
  setSourceFormReadonly(false);
  if (!s && !builtin) { syncSrcBlockLayout(); return; }
  $('src-detail-title').textContent = builtin ? L.localLibrary : (s.name || '曲源详情');
  $('src-detail-sub').textContent = builtin ? L.builtinTip
    : '可修改名称、目录、导入方式，或立即按此曲源导入曲库。';
  $('src-name').value = builtin ? L.localLibrary : (s.name || '');
  $('src-path').value = builtin ? (state.library_dir || '') : (s.path || '');
  $('src-netease').checked = builtin ? false : !!s.is_netease;
  $('src-enabled').checked = builtin ? true : !!s.enabled;
  document.querySelectorAll('input[name="src-mode"]').forEach((r) => {
    r.checked = builtin ? r.value === 'copy' : r.value === (s.mode || 'copy');
  });
  $('src-probe').textContent = '';
  if (builtin) { setSourceFormReadonly(true); syncSrcBlockLayout(); return; }
  syncSourceModeUI('src');
  syncSrcBlockLayout();
}

/** 复制 / 移动卡片状态：网易云来源强制复制并禁用「移动」。p = 'src'（详情）/ 'add-src'（添加窗口）。 */
function syncSourceModeUI(p) {
  const prefix = p || 'src';
  const forced = $(prefix + '-netease').checked;
  document.querySelectorAll(`input[name="${prefix}-mode"]`).forEach((r) => {
    r.disabled = forced && r.value === 'move';
    if (r.disabled) r.checked = false;
  });
  if (forced) {
    const copy = document.querySelector(`input[name="${prefix}-mode"][value="copy"]`);
    if (copy) copy.checked = true;
  }
  syncSourceOptionState(prefix);
}
function syncSourceOptionState(p) {
  const prefix = p || 'src';
  document.querySelectorAll(`#${prefix}-mode-options .imp-option`).forEach((label) => {
    const input = label.querySelector('input[type="radio"]');
    if (input) label.classList.toggle('is-checked', input.checked);
  });
}

/** 只读探测：选目录 / 手填路径后自动得出「是否网易云曲库 / 是否有 .ncm」。
 *  guide=true 时，添加曲源场景探测到网易云且无工具会引导配置 NCM 工具。 */
async function probeSourceDir(path, quiet, p, guide) {
  if (!path) return null;
  const prefix = p || 'src';
  const { data } = await api('/api/sources/probe', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path }),
  });
  if (!data || !data.ok) {
    if (!quiet) setHint((data && data.error) || '检测来源目录失败');
    return null;
  }
  const bits = [];
  if (!data.exists) bits.push('目录当前不可访问');
  else bits.push(data.is_netease
    ? '检测为网易云曲库：只加不删，强制复制' : '普通本地目录：可选复制或移动');
  if (data.ncm_found) bits.push(state.ncm_ready
    ? t('sources.ncm_found_ready', { _fallback: '发现 .ncm 加密文件，导入时会转换' })
    : t('sources.ncm_found_pending', { _fallback: '发现 .ncm 加密文件，未安装插件时会跳过并留在源目录' }));
  const el = $(prefix + '-probe');
  if (el) el.textContent = bits.join('；');
  if (guide && prefix === 'add-src') await maybeGuideNcmTool(data);
  return Object.assign({ path }, data);
}

/** 添加曲源探测到网易云且未安装 NCM 插件时引导；会话内只提示一次。 */
async function maybeGuideNcmTool(data) {
  if (state.ncm_ready || state.ncmGuideShown) return;
  if (!data.is_netease || !data.ncm_found) return;
  state.ncmGuideShown = true;
  const choice = await showDialog({
    title: t('sources.ncm_guide_title', { _fallback: '检测到网易云加密歌曲' }),
    text: t('sources.ncm_guide_text', {
      _fallback: '该目录包含 .ncm 加密文件。未安装 NCM 插件时会跳过并留在源目录，只导入普通音频；安装插件后导入曲源即可处理。',
    }),
    okText: t('sources.ncm_setup_now', { _fallback: '现在安装' }),
    cancelText: t('sources.ncm_setup_later', { _fallback: '暂不安装' }),
    cancelValue: 'later',
  });
  if (choice === 'later') return;
  closeSourceAddModal();
  openSettings();
  activateSettingsTab('plugins');
  refreshPluginsPage();
}

async function pickSourceDir(p) {
  const prefix = p || 'src';
  const { data } = await api('/api/pick-folder', { method: 'POST' });
  if (data && data.ok && !data.canceled && data.path) {
    $(prefix + '-path').value = data.path;
    const probe = await probeSourceDir(data.path, false, prefix, true);
    if (probe) {
      if (!$((prefix + '-name')).value.trim()) $(prefix + '-name').value = probe.suggested_name || '';
      if (prefix === 'add-src') $(prefix + '-netease').checked = !!probe.is_netease;
    }
    syncSourceModeUI(prefix);
  } else if (!data || !data.ok) {
    setHint((data && data.error) || '打开文件夹选择失败');
  }
}

/** 构造一份曲源 payload（两套表单共用同一组字段与端点）。 */
async function sourcePayload(prefix) {
  const path = $(prefix + '-path').value.trim();
  if (!path) { setHint('请先选择或输入来源目录'); return null; }
  const probe = await probeSourceDir(path, false, prefix);
  if (!probe || !probe.exists) { setHint('来源目录不存在或不可访问'); return null; }
  // 添加窗口以只读探测结果为准（编辑态保留用户手动更正的类型）。
  if (prefix === 'add-src') {
    $(prefix + '-netease').checked = !!probe.is_netease;
    syncSourceModeUI(prefix);
  }
  const netease = $(prefix + '-netease').checked;
  const modeInput = document.querySelector(`input[name="${prefix}-mode"]:checked`);
  return {
    name: $(prefix + '-name').value.trim() || probe.suggested_name || '新来源',
    path,
    is_netease: netease,
    mode: netease ? 'copy' : (modeInput ? modeInput.value : 'copy'),
    enabled: $(prefix + '-enabled').checked,
  };
}

/** 右侧详情「保存设置」：只 PUT 当前曲源。 */
async function saveSource() {
  // 内置「本地曲库」没有可写的配置记录，禁止走 sourcePayload / PUT。
  if (!state.sourcesCurrentId || state.sourcesCurrentId === LOCAL_LIBRARY_ID) return;
  const payload = await sourcePayload('src');
  if (!payload) return;
  const { data } = await api(`/api/sources/${encodeURIComponent(state.sourcesCurrentId)}`, {
    method: 'PUT', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '保存来源失败'); return; }
  state.sources = data.sources || state.sources;
  setHintOk('来源设置已保存');
  renderSourcesList();
  renderSourceForm();
}

/** 独立窗口「添加曲源」：POST 新曲源，成功后关闭窗口并在子页选中它。 */
async function saveAddSource() {
  const payload = await sourcePayload('add-src');
  if (!payload) return;
  const { data } = await api('/api/sources', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '添加曲源失败'); return; }
  state.sources = data.sources || state.sources;
  state.sourcesCurrentId = (data.source && data.source.id) || '';
  closeSourceAddModal();
  setHintOk('来源已添加');
  renderSourcesList();
  renderSourceForm();
  if (state.activeTab === 'sources') loadSourceGroups();
}

async function deleteSource() {
  // 内置「本地曲库」不可删除：按 id 先拦一道，find 不到真实记录再兜底。
  if (state.sourcesCurrentId === LOCAL_LIBRARY_ID) return;
  const s = state.sources.find((x) => x.id === state.sourcesCurrentId);
  if (!s) return;
  const L = sourceLabels();
  const yes = await showDialog({
    title: L.del,
    text: t('sources.delete_confirm', {
      name: s.name || s.path,
      _fallback: `确定删除「${s.name || s.path}」吗？只移除这条曲源记录，不会删除曲库或源目录里的任何文件。`,
    }),
    okText: L.del,
  });
  if (!yes) return;
  const { data } = await api(`/api/sources/${encodeURIComponent(s.id)}`, { method: 'DELETE' });
  if (!data || !data.ok) { setHint((data && data.error) || '删除来源失败'); return; }
  state.sources = data.sources || [];
  state.sourcesCurrentId = '';
  setHintOk('来源已删除');
  renderSourcesList();
  renderSourceForm();
  if (state.activeTab === 'sources') loadSourceGroups();
}

/** 按来源导入：网易云来源在后端强制复制（只加不删），普通来源按所选方式。
 *  v0.12.11：可显式传 id（串行更新复用）；opts.silent 表示由汇总条统一反馈。
 *  返回 { ok, imported, skipped, pending, held, failed } 供串行更新累计。 */
async function importSource(id, opts) {
  const sid = id || state.sourcesCurrentId;
  // 内置「本地曲库」不是上游目录，不能按曲源导入（避免曲库复制到自身）。
  if (sid === LOCAL_LIBRARY_ID) return { ok: false };
  const s = state.sources.find((x) => x.id === sid);
  if (!s) return { ok: false };
  if (!s.enabled) {
    if (!opts || !opts.silent) setHint('该曲源已停用，请先启用并保存');
    return { ok: false };
  }
  state.sourceStatus[s.id] = 'running';
  renderSourcesList();
  showProgressPanel(`曲源：${s.name || s.path}`);
  startProgressPolling();
  const { data } = await api(`/api/sources/${encodeURIComponent(s.id)}/import`, { method: 'POST' });
  finishProgressTask();
  state.sourceStatus[s.id] = (data && data.ok) ? 'done' : 'error';
  renderSourcesList();
  if (!data || !data.ok) {
    if (!opts || !opts.silent) setHint((data && data.error) || '曲源导入失败');
    return { ok: false };
  }
  if (!opts || !opts.silent) {
    // v0.16.2：单曲源导入也显示汇总反馈（串行更新由 updateAllEnabledSources 统一累计，不在此覆盖）
    state.sourceSummary = {
      done: 1, total: 1, imported: data.imported || 0, skipped: data.skipped || 0,
      filtered: data.filtered || 0, intra: data.intra_source_skipped || 0,
      upgraded: data.upgraded || 0, low: data.lower_quality_ignored || 0,
    };
    renderSourceSummary(true);
    setHintOk(`曲源导入完成：入库 ${data.imported}，跳过 ${data.skipped}，筛除 ${data.filtered || 0}，同源跳过 ${data.intra_source_skipped || 0}，升级 ${data.upgraded || 0}，低质忽略 ${data.lower_quality_ignored || 0}` +
      `${data.ncm_skipped ? `（含 ${data.ncm_skipped} 个加密文件，未配置工具）` : ''}${data.held ? `，重复挂起 ${data.held}` : ''}，失败 ${data.failed}`);
  }
  await refreshDuplicateBadge();
  if (data.held && (!opts || !opts.silent)) {
    // 待处理重复已是曲源总览子页的第 3 块：滚到该块并刷新，不再切分页。
    await loadSourcesDuplicates();
    const block = $('src-dup-section');
    if (block && block.scrollIntoView) block.scrollIntoView({ block: 'start' });
  }
  loadTracks($('search').value);
  // 归属可能已修正（替换 / 新入库），曲源页分组按需重算。
  if (state.activeTab === 'sources') loadSourceGroups();
  return {
    ok: true,
    imported: Number(data.imported) || 0,
    skipped: Number(data.skipped) || 0,
    pending: Number(data.pending) || 0,
    held: Number(data.held) || 0,
    failed: Number(data.failed) || 0,
  };
}

/** 来源管理内的「待处理重复」页：与原对比弹窗共用渲染 / 决策逻辑。 */
async function loadSourcesDuplicates() {
  const { data } = await api('/api/duplicates');
  if (!data || !data.ok) { setHint((data && data.error) || '读取重复组失败'); return; }
  const list = data.groups || [];
  state.duplicateGroups = list;
  state.duplicatesDir = data.pending_dir || state.duplicatesDir;
  state.duplicatesLogPath = data.log_path || '';
  updateDupBadge(data.count == null ? list.length : data.count);
  $('src-dup-count').textContent = list.length
    ? `共 ${list.length} 组待处理（全部未决组一次展示）` : '暂无待处理的重复组';
  $('src-dup-path').textContent = state.duplicatesDir
    ? `挂起文件：${state.duplicatesDir}（暂不处理不会移动文件）` : '';
  renderDuplicateGroups(list, $('src-dup-groups'));
}

// ---------- v0.12.11 曲源总览子页 / 添加曲源独立窗口 / rail 曲源页事件绑定 ----------
$('sources-close').addEventListener('click', closeSourceAddModal);
$('add-src-cancel').addEventListener('click', closeSourceAddModal);
$('sources-add').addEventListener('click', openSourceAddModal);
$('add-src-browse').addEventListener('click', () => pickSourceDir('add-src'));
$('add-src-netease').addEventListener('change', () => syncSourceModeUI('add-src'));
document.querySelectorAll('input[name="add-src-mode"]').forEach((r) =>
  r.addEventListener('change', () => syncSourceOptionState('add-src')));
$('add-src-path').addEventListener('blur', async () => {
  const p = $('add-src-path').value.trim();
  if (!p) return;
  const probe = await probeSourceDir(p, true, 'add-src');
  if (probe) {
    $('add-src-netease').checked = !!probe.is_netease;
    if (!$('add-src-name').value.trim()) $('add-src-name').value = probe.suggested_name || '';
    syncSourceModeUI('add-src');
  }
});
$('add-src-save').addEventListener('click', saveAddSource);

$('src-browse').addEventListener('click', () => pickSourceDir('src'));
$('src-netease').addEventListener('change', () => syncSourceModeUI('src'));
document.querySelectorAll('input[name="src-mode"]').forEach((r) =>
  r.addEventListener('change', () => syncSourceOptionState('src')));
$('src-path').addEventListener('blur', async () => {
  const p = $('src-path').value.trim();
  if (!p) return;
  await probeSourceDir(p, true, 'src');
});
$('src-save').addEventListener('click', saveSource);
$('src-delete').addEventListener('click', deleteSource);
$('src-import').addEventListener('click', () => importSource());
$('ov-update-all').addEventListener('click', updateAllEnabledSources);
$('sv-refresh').addEventListener('click', loadSourceGroups);
$('sv-list').addEventListener('click', async (e) => {
  const li = e.target.closest('li[data-id]');
  if (!li || li.classList.contains('pl-empty')) return;
  await loadSourceTracks(li.dataset.id);
});
$('sv-list').addEventListener('dblclick', (e) => {
  const li = e.target.closest('li[data-id]');
  if (!li || li.classList.contains('pl-empty')) return;
  const first = (state.sourceTracks || []).find((x) => Number(x.local_id) > 0);
  if (first) startPlayback(first, 'sources', { fullGroup: true });
  else setHint('该曲源暂无可播放的歌曲');
});
// ---------- 下载管理（队列监控 / 控制）----------
function openDownloads() {
  $('downloads-panel').hidden = false;
  loadDownloads();
  if (state.dlTimer) clearInterval(state.dlTimer);
  state.dlTimer = setInterval(loadDownloads, 1500);
}
function closeDownloads() {
  $('downloads-panel').hidden = true;
  if (state.dlTimer) { clearInterval(state.dlTimer); state.dlTimer = null; }
}
async function loadDownloads() {
  const { data } = await api('/api/downloads');
  if (data) renderDownloads(data.tasks || [], !!data.paused);
}
function renderDownloads(tasks, paused) {
  const body = $('dl-body');
  body.innerHTML = '';
  if (tasks.length === 0) {
    body.innerHTML = '<tr><td colspan="3" class="dl-empty">下载队列为空</td></tr>';
  }
  for (const t of tasks) {
    const tr = document.createElement('tr');
    const pct = t.size > 0 ? Math.round((t.progress / t.size) * 100) : 0;
    let ctrl = '';
    if (t.status === 'Queued' || t.status === 'Downloading') {
      ctrl = ` <button class="dl-row-btn" data-id="${esc(t.id)}" data-act="pause">暂停</button>`;
    } else if (t.status === 'Paused') {
      ctrl = ` <button class="dl-row-btn" data-id="${esc(t.id)}" data-act="start">继续</button>`;
    }
    const statusCell =
      esc(t.status_zh) +
      (t.status === 'Downloading' ? ` ${pct}%` : '') +
      ctrl +
      (t.error ? ` <span class="dl-err" title="${esc(t.error)}">⚠</span>` : '');
    tr.innerHTML =
      `<td>${esc(t.name)}<br><small>${esc(t.artist)}</small></td>` +
      `<td>${statusCell}</td>` +
      `<td class="dl-path">${t.target_path ? esc(t.target_path) : '—'}</td>`;
    body.appendChild(tr);
  }
  body.querySelectorAll('.dl-row-btn').forEach((b) =>
    b.addEventListener('click', () => {
      const id = b.dataset.id;
      const act = b.dataset.act;
      api('/api/downloads/' + (act === 'pause' ? 'pause' : 'start'), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id }),
      }).then(loadDownloads);
    })
  );
  $('dl-pause-all').disabled = paused;
  $('dl-start-all').disabled = !paused;
}
// 注：`btn-downloads` 入口已隐藏（下载功能保留窗口供未来复用），此处仅保留窗口内部控件逻辑。
$('dl-close').addEventListener('click', closeDownloads);
$('dl-pause-all').addEventListener('click', async () => {
  await api('/api/downloads/pause', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{}',
  });
  loadDownloads();
});
$('dl-start-all').addEventListener('click', async () => {
  await api('/api/downloads/start', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{}',
  });
  loadDownloads();
});
$('dl-clear-done').addEventListener('click', async () => {
  await api('/api/downloads/clear-completed', { method: 'POST' });
  loadDownloads();
});
$('dl-clear').addEventListener('click', async () => {
  if (!confirm('确定清空下载列表？进行中的下载将被取消。')) return;
  await api('/api/downloads/clear', { method: 'POST' });
  loadDownloads();
});

// ---------- 初始化向导（首次启动 / 曲库被移除）----------
// v0.12.0：第 3 步是通用「来源目录（可选）」；第 4 步 NCM 工具改为条件步骤，
// 仅当所选曲库 / 来源目录被判为网易云曲库、或探测到 .ncm 文件时才出现（wizNeedNcm）。
let wizStep = 1;
let wizNeedNcm = false;
state.wizSourceProbe = null;

function wizMaxStep() { return wizNeedNcm ? 4 : 3; }

function showInitWizard() {
  $('init-wizard').hidden = false;
  wizStep = 1;
  wizNeedNcm = false;
  state.wizSourceProbe = null;
  $('wiz-library-input').value = state.library_dir || '';
  // 预填曲库名称：已有则沿用，否则用目录名。
  if (state.library_name) {
    $('wiz-name-input').value = state.library_name;
  } else if (state.library_dir) {
    const parts = state.library_dir.split(/[\\/]/);
    $('wiz-name-input').value = parts[parts.length - 1] || '';
  } else {
    $('wiz-name-input').value = '';
  }
  $('wiz-netease-input').value = '';
  $('wiz-netease-status').textContent = '';
  $('wiz-netease').checked = false;
  const ncmReady = !!state.ncm_ready;
  $('wiz-ncm-status').textContent = ncmReady ? '插件状态：已安装' : '插件状态：未安装';
  const ncmBtn = $('wiz-ncm-install');
  if (ncmBtn) { ncmBtn.textContent = ncmReady ? '重新安装' : '安装插件'; ncmBtn.disabled = false; }
  renderWizStep();
}

function renderWizStep() {
  const max = wizMaxStep();
  document.querySelectorAll('.wizard-step').forEach((el) => {
    const n = Number(el.dataset.step);
    el.hidden = (n !== wizStep) || (n === 4 && !wizNeedNcm);
  });
  $('wiz-prev').hidden = (wizStep <= 1);
  $('wiz-next').hidden = (wizStep >= max);
  const atFinish = (wizStep >= max);
  const netease = !!$('wiz-netease').checked;
  // 网易云只加不删：底部合并为单个「导入」按钮（走 copy），隐藏移动按钮。
  $('wiz-finish-move').hidden = !atFinish || netease;
  $('wiz-finish-copy').hidden = !atFinish;
  $('wiz-finish-copy').textContent = netease ? '导入' : '复制导入';
  $('wiz-finish-copy').title = netease ? '保留源文件，只把副本与同名歌词加入曲库' : '保留源文件，只把副本加入曲库';
  // 第 3 步（来源目录）与条件第 4 步（NCM 工具）都可选，可直接完成。
  $('wiz-skip').hidden = (wizStep < 3);
}

/** 只读探测曲库目录 + 来源目录：仅以 .ncm 为网易云信号，目录名不再作为判据。
 *  探测失败静默降级为「未检测到」，不阻塞向导；只在点击 / 离开输入框时触发，不轮询。 */
async function wizProbeForNcm() {
  const libraryPath = $('wiz-library-input').value.trim();
  const sourcePath = $('wiz-netease-input').value.trim();
  const paths = [libraryPath, sourcePath].filter(Boolean);
  let hit = false;
  let sourceProbe = null;
  for (const p of paths) {
    const { data } = await api('/api/sources/probe', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: p }),
    });
    if (!data || !data.ok) continue;
    if (p === sourcePath && data.exists) sourceProbe = data;
    if (data.is_netease || data.ncm_found) hit = true;
  }
  if (sourceProbe) state.wizSourceProbe = sourceProbe;
  wizNeedNcm = hit;
  if (sourcePath && sourceProbe) {
    const netease = !!sourceProbe.ncm_found || !!sourceProbe.is_netease;
    $('wiz-netease').checked = netease;
    $('wiz-netease-status').textContent = netease
      ? '检测到 .ncm 文件：已按网易云曲库预勾选，只加不删、底部统一为「导入」（可手动取消勾选）'
      : '未检测到 .ncm 文件，可按需选择复制导入或移动导入';
  } else if (sourcePath) {
    $('wiz-netease-status').textContent = '已选择：' + sourcePath;
  }
  renderWizStep();
  return hit;
}

$('wiz-library-browse').addEventListener('click', async () => {
  const { data } = await api('/api/pick-library-directory', { method: 'POST' });
  if (data && data.ok && !data.canceled && data.path) {
    $('wiz-library-input').value = data.path;
    // 自动用目录名预填曲库名称（若用户尚未手动填写）。
    if (!$('wiz-name-input').value.trim()) {
      const parts = data.path.split(/[\\/]/);
      $('wiz-name-input').value = parts[parts.length - 1] || '';
    }
  }
});
$('wiz-netease-browse').addEventListener('click', async () => {
  const { data } = await api('/api/pick-folder', { method: 'POST' });
  if (data && data.ok && !data.canceled && data.path) {
    $('wiz-netease-input').value = data.path;
    await wizProbeForNcm();
  } else if (!data || !data.ok) {
    setHint((data && data.error) || '打开文件夹选择失败');
  }
});
$('wiz-netease-input').addEventListener('blur', async () => {
  if ($('wiz-netease-input').value.trim()) await wizProbeForNcm();
});
$('wiz-next').addEventListener('click', async () => {
  if (wizStep === 1 && !$('wiz-library-input').value.trim()) {
    setHint('请先指定曲库目录');
    return;
  }
  // 进入「来源目录」步骤前，先探测曲库目录本身是否含 .ncm / 是网易云曲库。
  if (wizStep === 2) await wizProbeForNcm();
  if (wizStep < wizMaxStep()) { wizStep++; renderWizStep(); }
});
$('wiz-prev').addEventListener('click', () => { if (wizStep > 1) { wizStep--; renderWizStep(); } });
$('wiz-skip').addEventListener('click', finishInit);
$('wiz-finish-copy').addEventListener('click', () => finishInit('copy'));
$('wiz-finish-move').addEventListener('click', () => finishInit('move'));
$('wiz-netease').addEventListener('change', () => {
  const netease = $('wiz-netease').checked;
  renderWizStep();
  $('wiz-netease-status').textContent = netease
    ? '网易云曲库按只加不删逻辑导入，底部统一为「导入」（保留源文件）'
    : '未检测到 .ncm 文件，可按需选择复制导入或移动导入';
});

/** 向导第 4 步专用安装：直接调插件安装端点，只局部更新状态，不触发 refreshStatus
 *  （否则 library_configured=false 时会 showInitWizard 重置向导、清空已填来源）。 */
async function wizInstallNcm() {
  const btn = $('wiz-ncm-install');
  const status = $('wiz-ncm-status');
  btn.disabled = true;
  setHint('正在安装 NCM 转换器插件…');
  try {
    const { data } = await api('/api/plugins/install', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: 'ncm' }),
    });
    if (!data || !data.ok) { setHint((data && data.error) || '安装失败'); return; }
    if (data.canceled) { setHint('已取消安装'); return; }
    state.ncm_ready = true;
    state.ncm_tool = data.tool_path || state.ncm_tool;
    status.textContent = '插件状态：已安装';
    btn.textContent = '重新安装';
    setHintOk('NCM 转换器插件已安装');
  } catch (e) {
    setHint('安装失败：' + (e && e.message ? e.message : e));
  } finally {
    btn.disabled = false;
  }
}
$('wiz-ncm-install').addEventListener('click', wizInstallNcm);

async function finishInit(requestedMode = 'copy') {
  const path = $('wiz-library-input').value.trim();
  if (!path) { setHint('请先指定曲库目录'); return; }
  const scheme = document.querySelector('input[name="wiz-scheme"]:checked').value;
  const is_netease = !!$('wiz-netease').checked;
  const mode = is_netease ? 'copy' : requestedMode;
  const library_name = $('wiz-name-input').value.trim();
  setHint('初始化曲库中…');
  const r = await api('/api/library-directory', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path, file_scheme: scheme, library_name }),
  });
  if (!r.data || !r.data.ok) { setHint((r.data && r.data.error) || '初始化失败'); return; }
  state.library_dir = r.data.library_dir;
  state.library_name = r.data.library_name || library_name;
  // 插件化后向导不再指定 NCM 工具路径；如需转换请到「设置 → 插件」安装。
  // 来源目录：统一走 /api/sources（后端自动检测网易云 + 强制复制），不再有独立的网易云导入界面。
  const sourcePath = $('wiz-netease-input').value.trim();
  let createdSource = null;
  if (sourcePath) {
    const created = await api('/api/sources', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: sourcePath, mode, is_netease, enabled: true }),
    });
    if (created.data && created.data.ok) {
      state.sources = created.data.sources || state.sources;
      createdSource = created.data.source || null;
      // 兼容旧端点 / CLI：网易云来源同时回填旧单值 [netease].library_dir（失败不阻塞初始化）。
      const isNetease = created.data.source && created.data.source.is_netease;
      if (isNetease) {
        const ne = await api('/api/netease/library-dir', {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ path: sourcePath }),
        });
        if (ne.data && ne.data.ok) state.netease_library_dir = ne.data.library_dir || sourcePath;
      }
    } else {
      setHint((created.data && created.data.error) || '来源目录写入失败，可稍后在「曲源」里添加');
    }
  }
  $('init-wizard').hidden = true;
  setHint('曲库已初始化，正在扫描…');
  await refreshStatus();
  const scan = await api('/api/scan', { method: 'POST' });
  let scanText = '';
  if (scan.data && scan.data.ok) {
    scanText = `（新增 ${scan.data.inserted}，更新 ${scan.data.updated}，移除 ${scan.data.removed}）`;
    maybeShowNcmOrganize(scan.data.ncm_found || [], true);
  }
  // v0.16.9：登记来源后必须真正导入一次，否则新曲库歌曲数为 0。
  let importText = '';
  if (createdSource && createdSource.id) {
    setHint('正在导入来源目录…');
    const imported = await importSource(createdSource.id, { silent: true });
    if (imported && imported.ok) importText = `，来源导入 ${imported.imported} 首`;
  }
  setHintOk(`初始化完成：${state.library_dir}${scanText}${importText}`);
  loadTracks($('search').value);
}

// ---------- 插件管理（L1 伪插件：当前仅 NCM 转换器）----------
function pluginNcm() {
  return (state.plugins || []).find((p) => p.id === 'ncm') || null;
}

/** 渲染插件页：左栏列表 + 右栏按钮（依安装/启用状态切换文案与显隐）。 */
function renderPluginsPage() {
  const list = $('plugin-list');
  if (!list) return;
  const p = pluginNcm();
  if (!p) {
    list.textContent = '未找到插件信息';
    list.classList.add('plugin-empty');
    // v0.17.0：插件信息缺失时同步隐藏右栏单项按钮，避免上一次渲染的显隐残留。
    ['plugin-toggle', 'plugin-remove'].forEach((id) => {
      const btn = $(id);
      if (btn) btn.hidden = true;
    });
    return;
  }
  list.classList.remove('plugin-empty');
  // v0.17.0：状态以 installed/enabled 为准（后端保证 ready = installed && enabled），
  // 不再出现「未安装」却显示「关闭插件」的口径冲突；meta 去掉重复的「状态：」。
  const statusCls = p.installed ? (p.enabled ? 'ok' : 'warn') : 'muted';
  const statusText = p.installed ? (p.enabled ? '已启用' : '已禁用') : '未安装';
  const version = p.version ? `v${p.version}` : '';

  const item = document.createElement('div');
  item.className = 'plugin-item';
  item.dataset.id = 'ncm';
  const head = document.createElement('div');
  head.className = 'plugin-item-head';
  const name = document.createElement('span');
  name.className = 'plugin-name';
  name.textContent = p.name || 'NCM 转换器';
  const badge = document.createElement('span');
  badge.className = 'plugin-badge ' + statusCls;
  badge.textContent = statusText;
  head.append(name, badge);
  const desc = document.createElement('div');
  desc.className = 'plugin-desc';
  desc.textContent = p.description || '';
  const meta = document.createElement('div');
  meta.className = 'plugin-meta';
  const metaBits = [];
  if (version) metaBits.push(version);
  if (!p.installed) metaBits.push('请安装后使用');
  meta.textContent = metaBits.join(' · ');
  item.append(head, desc, meta);
  list.replaceChildren(item);
  const toggleBtn = $('plugin-toggle');
  if (toggleBtn) {
    toggleBtn.hidden = !p.installed;
    toggleBtn.textContent = p.enabled ? '关闭插件' : '开启插件';
  }
  const removeBtn = $('plugin-remove');
  if (removeBtn) removeBtn.hidden = !p.installed;
  // v0.17.3：「转换积压 .ncm」按钮与角标已删除（pendingNcm 积压链路取消）；
  // 原始 .ncm 留在源曲源，用户通过「更新曲源」/「处理散落 .ncm」显式转换。
}

/** 重取插件列表。v0.17.0：返回是否成功，供「重新检测」区分提示；失败不渲染、不假成功。 */
async function refreshPluginsPage() {
  const { data } = await api('/api/plugins');
  if (!data || !data.ok) return false;
  state.plugins = data.plugins || [];
  renderPluginsPage();
  return true;
}

async function installPlugin() {
  // 安装只负责安装：不弹进度面板、不触发转换（v0.17.3 积压回补已删，转换走曲源更新 / 散落整理）。
  setHint('安装插件中…');
  const { data } = await api('/api/plugins/install', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id: 'ncm' }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '安装失败'); return; }
  if (data.canceled) { setHint('已取消安装'); return; }
  setHintOk('NCM 插件已安装：' + data.tool_path);
  await refreshStatus();
  await refreshPluginsPage();
}

async function togglePlugin() {
  const p = pluginNcm();
  if (!p) return;
  const { data } = await api('/api/plugins/toggle', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id: 'ncm', enabled: !p.enabled }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '操作失败'); return; }
  setHintOk(data.enabled ? 'NCM 插件已开启' : 'NCM 插件已关闭（工具文件保留）');
  await refreshStatus();
  await refreshPluginsPage();
}

async function removePlugin() {
  const confirmed = await showDialog({
    title: '删除 NCM 插件',
    text: '将物理删除 plugin 目录下的工具文件，且不可恢复。确定删除？',
    okText: '删除',
    cancelText: '取消',
    cancelValue: 'cancel',
  });
  if (confirmed === 'cancel') return;
  const { data } = await api('/api/plugins/remove', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id: 'ncm' }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '删除失败'); return; }
  setHintOk('NCM 插件已删除');
  await refreshStatus();
  await refreshPluginsPage();
}

async function openPluginDir() {
  // 走本地 REST → 后端 explorer.exe 打开 plugin_dir()，与其它「浏览 / 定位」按钮同范式；
  // 不再用 Tauri IPC plugin:shell|open 传本地路径（该路径曾静默无反应）。
  try {
    const { data } = await api('/api/plugins/open-dir', { method: 'POST' });
    if (data && data.ok) return;
    setHint((data && data.error) || '打开插件目录失败');
  } catch (e) {
    setHint('打开插件目录失败：' + (e && e.message ? e.message : e));
  }
}

$('plugin-install').addEventListener('click', installPlugin);
$('plugin-open-dir').addEventListener('click', openPluginDir);
$('plugin-refresh').addEventListener('click', async () => {
  // v0.17.0：后端列表查询已真实重扫 plugin 目录并落盘；成功才提示，失败回显错误。
  const ok = await refreshPluginsPage();
  if (ok) setHintOk('插件列表已重新检测');
  else setHint('插件列表检测失败，请稍后重试');
});
$('plugin-toggle').addEventListener('click', togglePlugin);
$('plugin-remove').addEventListener('click', removePlugin);

// ---------- 文件结构方案切换（重排）----------
async function applyScheme() {
  const sel = document.querySelector('input[name="set-scheme"]:checked');
  if (!sel) return;
  if (sel.value === state.file_scheme) { setHint('文件结构方案未变化'); return; }
  if (!confirm('切换方案将按新结构整体移动 / 重命名曲库文件，目录结构将大幅变化。重组中可暂停或结束（结束会整体回滚）。确定继续？')) return;

  // v0.15.1：先弹进度面板并开始轮询，再发阻塞式重组请求；共用导入进度面板与暂停/结束按钮。
  showProgressPanel('曲库重组', 'reorg');
  startProgressPolling();
  setHint('正在按新方案重组曲库…');
  const { ok, data } = await api('/api/reorganize', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ scheme: sel.value }),
  });
  if (!ok || !data || !data.ok) {
    finishProgressTask();
    setHint((data && data.error) || '重组失败');
    return;
  }
  if (data.cancelled) {
    setHint('重组已结束，曲库已回滚到原结构');
  } else {
    state.file_scheme = sel.value;
    setHintOk(`重组完成：已移动 ${data.moved} 个文件，清理 ${data.pruned || 0} 个空目录`);
    loadTracks($('search').value);
  }
  finishProgressTask();
}

// ================= 外观 / 主题 / 换肤 =================
// 主题方案以 **app 级配置文件 kqk-app.toml 的 [ui] 段为准**（与布局风格 / 列宽同源），
// localStorage 只作为首屏即时缓存 + 自定义皮肤图片本体（base64，体积大，不写 TOML）。
// 每次改动防抖 POST /api/ui-prefs 落盘；启动时由 /api/status 读回并覆盖本地缓存。
// #37（用户拍板推翻 FR-29 的「勿复活」条目）：主题色恢复。
// --primary / --primary-dark 由「手动主题色」或「皮肤背景自动提取主色」写入 :root 行内；
// 未设置时 active 不注入，让 classic / modern 各自 CSS 的明暗默认色生效（两套布局 1:1 保留）。
// 配置里的 accent / skin_accent 是后端既有字段（含 #RRGGBB 校验），无需改 Rust。
const APPEARANCE_KEY = 'kqk_appearance';

// FR-29 数值范围（与 config.rs 的常量保持一致；前端先钳一次，后端再钳一次）。
const LYRICS_OPACITY_MIN = 40, LYRICS_OPACITY_MAX = 100, LYRICS_OPACITY_DEFAULT = 82;
const SKIN_BLUR_MIN = 0, SKIN_BLUR_MAX = 40, SKIN_BLUR_DEFAULT = 18;
function clampLyricsOpacity(v) {
  const n = Number(v);
  return isFinite(n) ? Math.min(LYRICS_OPACITY_MAX, Math.max(LYRICS_OPACITY_MIN, Math.round(n))) : LYRICS_OPACITY_DEFAULT;
}
function clampSkinBlur(v) {
  const n = Number(v);
  return isFinite(n) ? Math.min(SKIN_BLUR_MAX, Math.max(SKIN_BLUR_MIN, Math.round(n))) : SKIN_BLUR_DEFAULT;
}

// Radix 色阶与映射统一来自 radix.js（index.html 首屏脚本与 core.js 共用，禁止各自复制色表）。
const RADIX = window.Radix || null;
if (!RADIX) console.error('radix.js 未加载：主题色无法套用 Radix 色阶');
function mapToRadixScale(hex, theme) { return RADIX ? RADIX.map(hex, theme) : null; }
function radixAccent(hex, theme) {
  if (RADIX) return RADIX.accent(hex, theme);
  return { scale: 'blue', shade: 9, vars: { '--primary': '#0090ff', '--primary-dark': '#0588f0' },
    primary: '#0090ff', dark: '#0588f0' };
}

// v0.13.0：主题色预设改用 Radix UI Colors 3.0.0 官方 9 阶（实心色）。
// 选中任一色系后由 radixAccent() 套用该色系明/暗各 12 阶全套令牌。
// UI STYLE：hex 不再硬编码复制，统一取自 radix.js 色表 9 阶（单一事实源）。
// v0.15.5：色板按暖→冷重排为 7 色；去掉纯灰 NEUTRAL，银灰 slate 改名「灰」，暖白 sand 色值不变。
// 顺序（暖→冷）：暖棕 tomato / 暖白 sand / 绿 green / 青 cyan / 蓝 blue / 紫 violet / 灰 slate。
// radix.js::map() 对精确 hex 命中色系，低饱和灰预设不会被近似规则误判为 gray。
const ACCENT_PRESETS = [
  { name: '暖棕', hex: RADIX.SCALES.tomato.light[8], scale: 'tomato' },
  { name: '暖白', hex: RADIX.SCALES.sand.light[8], scale: 'sand' },
  { name: '绿', hex: RADIX.SCALES.green.light[8], scale: 'green' },
  { name: '青', hex: RADIX.SCALES.cyan.light[8], scale: 'cyan' },
  { name: '蓝', hex: RADIX.SCALES.blue.light[8], scale: 'blue' },
  { name: '紫', hex: RADIX.SCALES.violet.light[8], scale: 'violet' },
  { name: '灰', hex: RADIX.SCALES.slate.light[8], scale: 'slate' },
];

// 预设皮肤：CSS 渐变（背景素材，非控件色，保留视觉）；预置主题色取自 Radix 色表 9 阶。
const SKIN_PRESETS = [
  { id: 'aurora', name: '极光', gradient: 'linear-gradient(135deg,#0f2027,#203a43,#2c5364)', accent: RADIX.SCALES.cyan.light[8] },
  { id: 'sakura', name: '樱粉', gradient: 'linear-gradient(135deg,#ffecd2,#fcb69f)', accent: RADIX.SCALES.tomato.light[8] },
  { id: 'mint', name: '薄荷', gradient: 'linear-gradient(135deg,#a1ffce,#faffd1)', accent: RADIX.SCALES.green.light[8] },
  { id: 'purple', name: '紫霞', gradient: 'linear-gradient(135deg,#667eea,#764ba2)', accent: RADIX.SCALES.violet.light[8] },
  { id: 'ocean', name: '深海', gradient: 'linear-gradient(135deg,#2193b0,#6dd5ed)', accent: RADIX.SCALES.blue.light[8] },
];

// 顶栏主题切换图标（SVG，避免依赖图片资源）。
const SUN_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>';
const MOON_SVG = '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/></svg>';

// 系统明暗偏好查询（跟随系统模式的数据源）。
const SYSTEM_DARK_QUERY = '(prefers-color-scheme: dark)';
function systemDark() {
  return typeof window.matchMedia === 'function' && window.matchMedia(SYSTEM_DARK_QUERY).matches;
}
/** 把设置值（light/dark/system）解析为实际渲染用的明暗主题。 */
function resolveTheme(mode) {
  if (mode === 'system') return systemDark() ? 'dark' : 'light';
  return mode === 'dark' ? 'dark' : 'light';
}

function defaultAppearance() {
  // 默认不注入行内强调色，由 modern.css 提供 Radix blue 出厂色；用户自选色仍按配置恢复。
  return {
    theme: 'light',
    accent: '',
    lyricsOpacity: LYRICS_OPACITY_DEFAULT,
    skinBlur: SKIN_BLUR_DEFAULT,
    skin: { enabled: false, type: null, src: null, gradient: null, name: null, accent: null },
  };
}
function loadAppearance() {
  try {
    const raw = localStorage.getItem(APPEARANCE_KEY);
    if (!raw) return defaultAppearance();
    const a = Object.assign(defaultAppearance(), JSON.parse(raw));
    if (!a.skin) a.skin = defaultAppearance().skin;
    return a;
  } catch (e) { return defaultAppearance(); }
}

// 用户本次会话是否手动改过外观：改过之后不再被后续 /api/status 刷新覆盖（本地即最新）。
let appearanceTouched = false;

// 只写本地缓存（不落盘、不标脏）：供「从配置还原」使用，避免产生回环 POST。
function writeAppearanceLocal(a) {
  try { localStorage.setItem(APPEARANCE_KEY, JSON.stringify(a)); } catch (e) { /* 容量超限忽略 */ }
}

// 外观对象 → 配置字段。自定义图片的 data URL 不参与，只留可复现方案的轻量信息。
function appearanceToConfig(a) {
  const s = a.skin || defaultAppearance().skin;
  const preset = (s.enabled && s.type === 'preset')
    ? SKIN_PRESETS.find((p) => p.gradient === s.gradient)
    : null;
  const type = s.enabled ? (s.type === 'custom' ? 'custom' : (preset ? 'preset' : '')) : '';
  return {
    theme: ['light', 'system', 'dark'].includes(a.theme) ? a.theme : 'system',
    accent: a.accent || '',
    skin_type: type,
    skin_preset: (type === 'preset' && preset) ? preset.id : '',
    // #37：皮肤激活时把自动提取的主色落盘，重启后首屏脚本可提前预设，无需重新解码图片。
    skin_accent: (s.enabled && s.accent) ? s.accent : '',
    // 后端 appearance 段是部分更新语义，两个可调数值随外观一起落盘。
    lyrics_opacity: clampLyricsOpacity(a.lyricsOpacity),
    skin_blur: clampSkinBlur(a.skinBlur),
  };
}

// 配置字段 → 外观对象。自定义皮肤图片本体从本地缓存找回；找不到则优雅退化为「无皮肤」。
function appearanceFromConfig(cfg) {
  const base = defaultAppearance();
  if (!cfg || typeof cfg !== 'object') return base;
  const local = loadAppearance();
  const theme = ['light', 'system', 'dark'].includes(cfg.theme) ? cfg.theme : 'system';
  const preset = SKIN_PRESETS.find((p) => p.id === cfg.skin_preset);
  let skin = Object.assign({}, base.skin);
  if (cfg.skin_type === 'preset' && preset) {
    skin = {
      enabled: true, type: 'preset', gradient: preset.gradient,
      accent: cfg.skin_accent || preset.accent, name: preset.name, src: null,
    };
  } else if (cfg.skin_type === 'custom' && local.skin && local.skin.type === 'custom' && local.skin.src) {
    skin = Object.assign({}, local.skin, {
      enabled: true, accent: cfg.skin_accent || local.skin.accent || null,
    });
  }
  // 旧版 localStorage 里的驼峰键名也兼容；配置注入用下划线键名。
  return {
    theme,
    accent: cfg.accent || null,
    lyricsOpacity: clampLyricsOpacity(cfg.lyrics_opacity === undefined ? local.lyricsOpacity : cfg.lyrics_opacity),
    skinBlur: clampSkinBlur(cfg.skin_blur === undefined ? local.skinBlur : cfg.skin_blur),
    skin,
  };
}

// 防抖：自定义取色器 / 色板连续触发时合并成一次落盘，避免刷屏写配置文件。
let appearanceSaveTimer = 0;
function persistAppearanceToConfig(a) {
  clearTimeout(appearanceSaveTimer);
  appearanceSaveTimer = setTimeout(() => {
    api('/api/ui-prefs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ appearance: appearanceToConfig(a) }),
    });
  }, 200);
}

function saveAppearance(a) {
  writeAppearanceLocal(a);
  appearanceTouched = true;
  persistAppearanceToConfig(a);
}

// 启动 / 刷新状态时与配置文件对齐：
//   saved=true  → 配置为准，覆盖本地缓存后重新应用；
//   saved=false → 旧版本从未写外观，把 localStorage 里的老偏好一次性迁移进配置（不反向覆盖）。
function syncAppearanceFromStatus(cfg) {
  // 布局：即使外观尚未「被用户触碰」也照常应用（boot 时已按注入值预设，这里与状态对齐）。
  if (cfg && cfg.layout_skin) applyLayoutSkin(cfg.layout_skin, false);
  // 列表密度：同理，启动即与状态对齐（默认 comfortable）。
  if (cfg && cfg.list_density) applyListDensity(cfg.list_density, false);
  if (!cfg || appearanceTouched) return;
  if (cfg.saved) {
    writeAppearanceLocal(appearanceFromConfig(cfg));
    applyAppearance();
    renderAppearanceUI();
  } else {
    persistAppearanceToConfig(loadAppearance());
  }
}

// ---------- #37：主题色工具（FR-29 前实现，按用户拍板恢复）----------
function hexToRgb(hex) {
  let s = String(hex).replace('#', '');
  if (s.length === 3) s = s.split('').map((c) => c + c).join('');
  const n = parseInt(s, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
function rgbToHex(r, g, b) {
  const h = (x) => ('0' + Math.round(Math.max(0, Math.min(255, x))).toString(16)).slice(-2);
  return '#' + h(r) + h(g) + h(b);
}
function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let h = 0, s = 0, l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    switch (max) {
      case r: h = (g - b) / d + (g < b ? 6 : 0); break;
      case g: h = (b - r) / d + 2; break;
      case b: h = (r - g) / d + 4; break;
    }
    h /= 6;
  }
  return [h, s, l];
}
function hslToRgb(h, s, l) {
  let r, g, b;
  if (s === 0) { r = g = b = l; }
  else {
    const hue2rgb = (p, q, t) => {
      if (t < 0) t += 1; if (t > 1) t -= 1;
      if (t < 1 / 6) return p + (q - p) * 6 * t;
      if (t < 1 / 2) return q;
      if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
      return p;
    };
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    r = hue2rgb(p, q, h + 1 / 3); g = hue2rgb(p, q, h); b = hue2rgb(p, q, h - 1 / 3);
  }
  return [r * 255, g * 255, b * 255];
}
/** 从图片取稳定主色：保留色相，规范化饱和度/明度，再近似映射到最近 Radix 色阶（选图时一次性执行）。 */
function extractDominantColor(img, theme) {
  const c = document.createElement('canvas');
  const w = (c.width = 64), h = (c.height = 64);
  const ctx = c.getContext('2d');
  try { ctx.drawImage(img, 0, 0, w, h); } catch (e) { return null; }
  const data = ctx.getImageData(0, 0, w, h).data;
  let r = 0, g = 0, b = 0, count = 0;
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 125) continue; // 跳过透明像素
    r += data[i]; g += data[i + 1]; b += data[i + 2]; count++;
  }
  if (!count) return null;
  const avgR = Math.round(r / count), avgG = Math.round(g / count), avgB = Math.round(b / count);
  let [hue, sat] = rgbToHsl(avgR, avgG, avgB);
  if (sat < 0.12) {
    // UI STYLE：近灰阶按 Radix 规范映射到 gray 对应明度阶（非 blue）；map() 内部已按 sat 判定。
    const mapped = mapToRadixScale(rgbToHex(avgR, avgG, avgB), theme);
    if (mapped) return mapped.hex;
    const steps = RADIX.NEUTRAL[theme === 'dark' ? 'dark' : 'light'];
    const lig = rgbToHsl(avgR, avgG, avgB)[2];
    let best = 0, bestD = Infinity;
    for (let i = 0; i < steps.length; i++) {
      const d = Math.abs(rgbToHsl.apply(null, RADIX.hexToRgb(steps[i]))[2] - lig);
      if (d < bestD) { bestD = d; best = i; }
    }
    return steps[best];
  }
  sat = 0.58;
  const lig = theme === 'dark' ? 0.62 : 0.48;
  const [nr, ng, nb] = hslToRgb(hue, sat, lig);
  const mapped = mapToRadixScale(rgbToHex(nr, ng, nb), theme);
  // 映射不可用时回退到该主题 Radix blue 9 阶出厂色，不吐自调 hex。
  return mapped ? mapped.hex : RADIX.SCALES.blue[theme === 'dark' ? 'dark' : 'light'][8];
}
/** 只接受 #RRGGBB（与后端 is_hex_color 同口径），避免脏数据写进 CSS 变量。 */
function isAccentHex(v) { return typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v); }

// 将用户选择的图片压缩为适合做背景的数据 URL（控制 localStorage 体积）。
function fileToSkinDataURL(file, maxDim, cb) {
  const reader = new FileReader();
  reader.onload = () => {
    const img = new Image();
    img.onload = () => {
      const scale = Math.min(1, maxDim / Math.max(img.width, img.height));
      const c = document.createElement('canvas');
      c.width = Math.max(1, Math.round(img.width * scale));
      c.height = Math.max(1, Math.round(img.height * scale));
      c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
      cb(c.toDataURL('image/jpeg', 0.82));
    };
    img.onerror = () => cb(null);
    img.src = reader.result;
  };
  reader.onerror = () => cb(null);
  reader.readAsDataURL(file);
}

// ---------- 原生窗口标题栏颜色同步 ----------
// WebView 内容里的 .topbar 用 --primary 渐变着色，而窗口标题栏由 Windows DWM 绘制。
// #37：每次外观变化都上报当前生效的 --primary（含主题色/皮肤自动取色），保持同色。
// CLI serve / 浏览器预览下没有原生窗口，请求失败静默忽略。
function syncNativeTitlebar(hex, effective) {
  try {
    fetch('/api/appearance/titlebar', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ color: hex, dark: effective === 'dark' }),
    }).catch(() => {});
  } catch (e) { /* 非 GUI 环境忽略 */ }
}

// ---------- 应用外观 ----------
function applyAppearance() {
  const a = loadAppearance();
  const root = document.documentElement;
  const eff = resolveTheme(a.theme);
  root.setAttribute('data-theme', eff);
  root.setAttribute('data-theme-mode', a.theme); // 保留设置值，便于诊断与后续样式扩展

  // v0.13.0：强调色经 Radix 映射后成套写入。皮肤激活时优先用皮肤提取色，否则用手动主题色；
  // 两者都空则清掉行内覆盖，让 modern.css 的 Radix blue 默认色生效。
  const accent = (a.skin && a.skin.enabled && a.skin.accent)
    ? a.skin.accent
    : (a.accent || '');
  const radixTokens = ['--primary', '--primary-dark', '--primary-text', '--primary-strong-text',
    '--primary-subtle', '--primary-border', '--primary-glow'];
  for (let i = 1; i <= 12; i++) radixTokens.push('--radix-' + i);
  if (isAccentHex(accent)) {
    const radix = radixAccent(accent, eff);
    Object.keys(radix.vars).forEach((k) => root.style.setProperty(k, radix.vars[k]));
    root.style.setProperty('--primary', radix.primary);
    root.style.setProperty('--primary-dark', radix.dark);
    root.style.setProperty('--primary-text', radix.text);
    root.style.setProperty('--primary-strong-text', radix.strongText);
    root.style.setProperty('--primary-subtle', radix.subtle);
    root.style.setProperty('--primary-border', radix.border);
    root.style.setProperty('--primary-glow', radix.glow);
    root.setAttribute('data-radix-scale', radix.scale);
    root.classList.add('has-accent'); // rail / 侧栏染色 + 派生令牌开关（首屏脚本同步预设）
  } else {
    radixTokens.forEach((k) => root.style.removeProperty(k));
    root.removeAttribute('data-radix-scale');
    root.classList.remove('has-accent');
  }

  // 两个可调数值：applyAppearance() 是运行期唯一入口，首屏脚本只提前预设同值防闪烁。
  const lyrics = clampLyricsOpacity(a.lyricsOpacity);
  const blur = clampSkinBlur(a.skinBlur);
  root.style.setProperty('--lyrics-alpha', lyrics + '%');
  root.style.setProperty('--skin-blur', blur + 'px');

  // 应用皮肤（底层模糊背景图 + 玻璃）。
  const body = document.body;
  if (a.skin && a.skin.enabled && (a.skin.src || a.skin.gradient)) {
    body.classList.add('skin-active');
    body.style.setProperty('--skin-image', a.skin.gradient ? a.skin.gradient : `url("${a.skin.src}")`);
  } else {
    body.classList.remove('skin-active');
    body.style.removeProperty('--skin-image');
  }

  // 顶栏图标：暗色时显示太阳（点一下回浅色），亮色时显示月亮。
  const btn = $('btn-theme');
  if (btn) {
    btn.innerHTML = eff === 'dark' ? SUN_SVG : MOON_SVG;
    btn.title = a.theme === 'system' ? '切换明暗主题（当前：跟随系统）' : '切换明暗主题';
  }

  // 标题栏与顶栏同色：读当前计算出的 --primary（含各皮肤默认色），避免写死品牌蓝。
  const titleColor = getComputedStyle(root).getPropertyValue('--primary').trim() || '#0090ff';
  syncNativeTitlebar(titleColor, eff);
}

function setSkin(skin) {
  const a = loadAppearance();
  a.skin = skin
    ? Object.assign({ enabled: true }, skin)
    : { enabled: false, type: null, src: null, gradient: null, name: null, accent: null };
  saveAppearance(a);
  applyAppearance();
  renderAppearanceUI();
}

// #33：外观页「布局 / 列表密度」换成与主题模式同款 .seg 分段控件后，
// 回显统一走这里；applyLayoutSkin / applyListDensity / renderAppearanceUI 三处共用，
// 避免只换 DOM 却漏掉选中态同步。dataKey 传 'skin' / 'density'（对应 data-* 属性）。
function syncSegGroup(groupId, dataKey, value) {
  const g = $(groupId);
  if (!g) return;
  g.querySelectorAll('.seg-btn').forEach((b) => {
    const on = b.dataset[dataKey] === value;
    b.classList.toggle('active', on);
    b.setAttribute('aria-pressed', String(on));
  });
}

// ---------- 渲染外观设置 UI ----------
function renderAppearanceUI() {
  if (!$('ap-skins')) return;
  const a = loadAppearance();

  // 主题模式分段控件（按设置值高亮，system 为中间项）
  document.querySelectorAll('#ap-theme-seg .seg-btn').forEach((b) => {
    b.classList.toggle('active', b.dataset.theme === a.theme);
  });
  // #33：布局 / 列表密度分段控件回显（layoutSkin / listDensity 在 boot 时已设置）。
  syncSegGroup('ap-layout-skin', 'skin', layoutSkin);
  syncSegGroup('ap-list-density', 'density', listDensity);
  const hint = $('ap-theme-hint');
  if (hint) {
    hint.textContent = a.theme === 'system'
      ? `跟随系统：当前系统为${systemDark() ? '深色' : '浅色'}，系统主题变化会自动同步`
      : (a.theme === 'dark' ? '已固定为深色主题' : '已固定为浅色主题');
  }

  // #37：主题色色板（仅首次构建）+ 回显。皮肤激活时色板不高亮（主色由皮肤决定）。
  const sw = $('ap-accent-swatches');
  if (sw && !sw.children.length) {
    ACCENT_PRESETS.forEach((p) => {
      const el = document.createElement('button');
      el.className = 'swatch'; el.type = 'button'; el.title = p.name;
      el.style.background = p.hex; el.dataset.hex = p.hex;
      el.addEventListener('click', () => {
        const aa = loadAppearance();
        aa.accent = p.hex;
        if (aa.skin) aa.skin.enabled = false; // 手动选色视为退出皮肤派生
        saveAppearance(aa); applyAppearance(); renderAppearanceUI();
      });
      sw.appendChild(el);
    });
  }
  const curAccent = (a.skin && a.skin.enabled && a.skin.accent)
    ? a.skin.accent : (a.accent || RADIX.SCALES.blue.light[8]);
  if (sw) sw.querySelectorAll('.swatch').forEach((el) => {
    el.classList.toggle('active', !(a.skin && a.skin.enabled) &&
      !!curAccent && curAccent.toLowerCase() === el.dataset.hex.toLowerCase());
  });
  const cc = $('ap-accent-custom');
  if (cc) cc.value = isAccentHex(a.accent) ? a.accent : '#0090ff';

  // 皮肤缩略图（仅首次构建）
  const sk = $('ap-skins');
  if (sk && !sk.children.length) {
    const none = document.createElement('button');
    none.className = 'skin-thumb none'; none.type = 'button'; none.title = '无皮肤';
    none.textContent = '无';
    none.addEventListener('click', () => setSkin(null));
    sk.appendChild(none);
    SKIN_PRESETS.forEach((p) => {
      const el = document.createElement('button');
      el.className = 'skin-thumb'; el.type = 'button'; el.title = p.name;
      el.style.background = p.gradient; el.dataset.id = p.id;
      el.addEventListener('click', () => setSkin({ type: 'preset', gradient: p.gradient, accent: p.accent, name: p.name }));
      sk.appendChild(el);
    });
  }
  document.querySelectorAll('#ap-skins .skin-thumb').forEach((el) => {
    const on = a.skin && a.skin.enabled;
    if (el.classList.contains('none')) el.classList.toggle('active', !on);
    else el.classList.toggle('active', on && a.skin.type === 'preset' && a.skin.gradient === el.style.background);
  });

  // 当前皮肤说明（#37：恢复自动提取主题色的提示）。
  const cur = $('ap-skin-current');
  if (cur) {
    if (a.skin && a.skin.enabled) {
      const label = a.skin.type === 'preset' ? ('预设「' + (a.skin.name || '') + '」') : '自定义图片';
      cur.textContent = `当前皮肤：${label}，主题色已根据背景自动设为 ${a.skin.accent || '-'}（模糊 ${clampSkinBlur(a.skinBlur)}px）`;
    } else {
      cur.textContent = '当前：无皮肤（使用上方主题色，模糊滑块在选择皮肤后生效）';
    }
  }

  // FR-29 两个数值滑块：既回显配置读回值，也回显拖动过程中的即时值。
  const lo = $('ap-lyrics-opacity'), loOut = $('ap-lyrics-opacity-value');
  const lv = clampLyricsOpacity(a.lyricsOpacity);
  if (lo) lo.value = String(lv);
  if (loOut) loOut.textContent = lv + '%';
  const sb = $('ap-skin-blur'), sbOut = $('ap-skin-blur-value');
  const bv = clampSkinBlur(a.skinBlur);
  if (sb) sb.value = String(bv);
  if (sbOut) sbOut.textContent = bv + 'px';
  // 交互增强 b 级：托盘开关回显。
  const ctt = $('ap-close-to-tray'), mtt = $('ap-minimize-to-tray');
  if (ctt) ctt.checked = !!state.closeToTray;
  if (mtt) mtt.checked = !!state.minimizeToTray;
}

// ---------- 布局皮肤（v0.15.0 起唯一 modern）----------
// 首屏已固定加载 modern.css 并给 <html> 加 skin-modern；这里只保留状态对齐与持久化入口。
// 旧配置 layout_skin=classic 一律归一到 modern，皮肤层与底层背景皮肤（skin_type）正交。
let layoutSkin = 'modern';
// 「共 N 首」固定在列表筛选条右侧；刷新图标已移除，只挂载计数，勿依赖其存在导致计数错位。
function syncToolbarExtras() {
  const count = $('track-count');
  const bar = $('track-filter-bar');
  if (!count || !bar) return;
  if (count.parentNode !== bar) bar.appendChild(count);
}

function applyLayoutSkin(name, persist) {
  // 单皮肤：忽略传入值，恒为 modern（兼容旧配置里残留的 classic）。
  // skin-modern 类由 index.html 首屏脚本固定加上，这里不再 add —— 否则会覆盖 demo 预览
  // 为测基础层收起条交互而临时移除的 class（base.css 保留原规则）。
  layoutSkin = 'modern';
  document.body.dataset.skin = 'modern';
  syncToolbarExtras();
  // 仅 modern（有 skin-modern 类）强制整页封面墙；demo 预览移除该类时不干预默认视图。
  if (document.documentElement.classList.contains('skin-modern')) setAlbumView('grid');
  syncSegGroup('ap-layout-skin', 'skin', 'modern');
  applyPaneLayout();
  if ($('detail-pane')) syncDetailVisibility();
  if (persist) {
    fetch('/api/ui-prefs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ layout_skin: 'modern' }),
    }).catch(() => {});
  }
}

// ---------- 列表密度切换（comfortable / compact）----------
// 纯前端布局呈现（行高 / 内边距），不改数据；落 app 级 [ui].list_density，重启沿用。
let listDensity = 'comfortable';
function applyListDensity(name, persist) {
  if (name !== 'comfortable' && name !== 'compact') name = 'comfortable';
  listDensity = name;
  document.body.classList.toggle('density-compact', name === 'compact');
  syncSegGroup('ap-list-density', 'density', name);
  if (persist) {
    fetch('/api/ui-prefs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ list_density: name }),
    }).catch(() => {});
  }
}

// ---------- 绑定外观事件（仅一次） ----------
function initAppearanceEvents() {
  document.querySelectorAll('#ap-theme-seg .seg-btn').forEach((b) => {
    b.addEventListener('click', () => {
      const a = loadAppearance();
      a.theme = b.dataset.theme;
      saveAppearance(a); applyAppearance(); renderAppearanceUI();
    });
  });
  // #37：自定义取色器。手动选色即退出皮肤派生，走与色板相同的保存链路。
  const accentCustom = $('ap-accent-custom');
  if (accentCustom) accentCustom.addEventListener('input', (e) => {
    const a = loadAppearance();
    a.accent = e.target.value;
    if (a.skin) a.skin.enabled = false;
    saveAppearance(a); applyAppearance(); renderAppearanceUI();
  });
  // FR-29：两个数值滑块。input 连续触发，但 saveAppearance() 内部已有 200ms 防抖，
  // 一次拖动最多产生一次 /api/ui-prefs 写盘。这里不调用 renderAppearanceUI()，
  // 避免把滑块 value 重设成 step 对齐后的值导致拖动中跳动；只同步右侧读数。
  const bindRange = (inputId, outputId, key, clamp, suffix) => {
    const input = $(inputId), out = $(outputId);
    if (!input) return;
    input.addEventListener('input', (e) => {
      const a = loadAppearance();
      const v = clamp(e.target.value);
      a[key] = v;
      saveAppearance(a); applyAppearance();
      if (out) out.textContent = v + suffix;
    });
  };
  bindRange('ap-lyrics-opacity', 'ap-lyrics-opacity-value', 'lyricsOpacity', clampLyricsOpacity, '%');
  bindRange('ap-skin-blur', 'ap-skin-blur-value', 'skinBlur', clampSkinBlur, 'px');

  // #33：布局 / 列表密度分段按钮（与主题模式同款），点击委托到各自的 seg 容器。
  const layoutSeg = $('ap-layout-skin');
  if (layoutSeg) layoutSeg.addEventListener('click', (e) => {
    const b = e.target.closest('.seg-btn');
    if (b && layoutSeg.contains(b)) applyLayoutSkin(b.dataset.skin, true);
  });
  const densitySeg = $('ap-list-density');
  if (densitySeg) densitySeg.addEventListener('click', (e) => {
    const b = e.target.closest('.seg-btn');
    if (b && densitySeg.contains(b)) applyListDensity(b.dataset.density, true);
  });
  // 交互增强 b 级：托盘开关（关闭→托盘 / 最小化→托盘）。
  const ctt = $('ap-close-to-tray');
  if (ctt) ctt.addEventListener('change', () => {
    state.closeToTray = ctt.checked;
    savePanePrefs({ close_to_tray: ctt.checked });
  });
  const mtt = $('ap-minimize-to-tray');
  if (mtt) mtt.addEventListener('change', () => {
    state.minimizeToTray = mtt.checked;
    savePanePrefs({ minimize_to_tray: mtt.checked });
  });

  const pick = $('ap-skin-pick');
  if (pick) pick.addEventListener('click', () => { const f = $('ap-skin-file'); if (f) f.click(); });
  const file = $('ap-skin-file');
  if (file) file.addEventListener('change', (e) => {
    const f = e.target.files && e.target.files[0];
    if (!f) return;
    // #37：压缩图片作底层背景，并从同一张已解码的图里 64×64 canvas 取一次主色。
    fileToSkinDataURL(f, 1600, (dataUrl) => {
      if (!dataUrl) { setHint('图片读取失败'); return; }
      const img = new Image();
      img.onload = () => {
        const eff = resolveTheme(loadAppearance().theme);
        const accent = extractDominantColor(img, eff) || '#0090ff';
        setSkin({ type: 'custom', src: dataUrl, accent, name: '自定义' });
        setHintOk('已应用自定义皮肤，主题色已根据图片自动调整');
      };
      img.onerror = () => { setHint('图片解码失败'); };
      img.src = dataUrl;
    });
    e.target.value = '';
  });
  const clear = $('ap-skin-clear');
  if (clear) clear.addEventListener('click', () => setSkin(null));
  const themeBtn = $('btn-theme');
  if (themeBtn) themeBtn.addEventListener('click', () => {
    const a = loadAppearance();
    // 跟随系统时一键切到与当前系统相反的显式主题；已固定时浅/暗互切。
    a.theme = a.theme === 'system'
      ? (systemDark() ? 'light' : 'dark')
      : (a.theme === 'dark' ? 'light' : 'dark');
    saveAppearance(a); applyAppearance(); renderAppearanceUI();
  });

  // 系统明暗偏好变化时，仅在「跟随系统」模式下重新解析并应用（含原生标题栏同步）。
  if (typeof window.matchMedia === 'function') {
    const mq = window.matchMedia(SYSTEM_DARK_QUERY);
    const onSystemThemeChange = () => {
      if (loadAppearance().theme !== 'system') return;
      applyAppearance();
      renderAppearanceUI();
    };
    if (typeof mq.addEventListener === 'function') mq.addEventListener('change', onSystemThemeChange);
    else if (typeof mq.addListener === 'function') mq.addListener(onSystemThemeChange); // 旧 WebView2 兜底
  }
}

// ---------- FR-24：侧栏宽度 / 收缩 / 音量 / 双击整组播放 ----------
const PANE_WIDTH_LIMITS = {
  sidebar: { min: 96, max: 320, fallback: 144 },
  group: { min: 160, max: 480, fallback: 230 },
};
function clampPaneWidth(v, which) {
  const L = PANE_WIDTH_LIMITS[which];
  return (isFinite(v) && v >= L.min && v <= L.max) ? Math.round(v) : L.fallback;
}
// 每栏唯一的贯穿全高细条：横向布局用左右箭头；纵向布局的内联详情条是水平条，改上下方向。
// 细条相对所属栏绝对定位、贴固定间隙 → 折叠前后水平位置不变。
// 同时按当前展开 / 收起态写入即时悬浮提示（CSS ::after 读 data-tip）与读屏标签。
function setPaneTip(el, text) {
  if (!el) return;
  el.setAttribute('data-tip', text);
  el.setAttribute('aria-label', text);
}
function updatePaneHandleArrows() {
  const sh = $('handle-sidebar');
  if (sh) {
    sh.textContent = paneCollapsed('sidebar') ? '›' : '‹';
    setPaneTip(sh, paneCollapsed('sidebar') ? '展开一级菜单' : '收起一级菜单');
  }
  document.querySelectorAll('.pane-handle.group-handle').forEach((b) => {
    b.textContent = paneCollapsed('group') ? '›' : '‹';
    setPaneTip(b, paneCollapsed('group') ? '展开二级列表' : '收起二级列表');
  });
  const dh = $('handle-detail');
  if (dh) {
    const inline = document.body.classList.contains('detail-inline');
    dh.textContent = inline
      ? (paneCollapsed('detail') ? '⌄' : '⌃')
      : (paneCollapsed('detail') ? '‹' : '›');
    setPaneTip(dh, (paneCollapsed('detail') ? '展开' : '收起') + (inline ? '歌曲详情' : '歌曲信息栏'));
  }
}
// 统一把内存状态投射到 DOM：宽度用 CSS 变量（body class 决定收起态），按钮常驻可展开。
// v0.15.0：唯一 modern 布局隐藏全部伸缩条，恒展开（state / 持久化字段保留不动，仅为兼容旧配置）。
// 按 <html> 的 skin-modern 类判断：产品单皮肤下恒有该类 → 恒展开；demo 预览移除该类可回归
// 基础样式层的收起条交互（base.css 保留原规则，仅供历史几何断言测试）。
function paneCollapsed(which) {
  if (document.documentElement.classList.contains('skin-modern')) return false;
  if (which === 'sidebar') return !!state.sidebarCollapsed;
  if (which === 'group') return !!state.groupCollapsed;
  return !!state.detailCollapsed;
}
function applyPaneLayout() {
  document.documentElement.style.setProperty('--sidebar-w', state.sidebarWidth + 'px');
  document.documentElement.style.setProperty('--group-w', state.groupWidth + 'px');
  document.body.classList.toggle('sidebar-collapsed', paneCollapsed('sidebar'));
  document.body.classList.toggle('group-collapsed', paneCollapsed('group'));
  // FR-25：歌曲信息栏收缩态（含封面），与两侧栏共用同一套持久化语义。
  document.body.classList.toggle('detail-collapsed', paneCollapsed('detail'));
  updatePaneHandleArrows();
}
// v0.16.18：非 UI 偏好已从 [ui] 拆出，按 patch 键路由到专用端点；真 UI 仍走 /api/ui-prefs。
const PLAYBACK_PREF_KEYS = new Set(['auto_playlist_count', 'volume', 'muted']);
const WINDOW_PREF_KEYS = new Set(['close_to_tray', 'minimize_to_tray']);
function postPrefs(path, patch) {
  return api(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(patch),
  }).then(({ data }) => {
    if (!data || !data.ok) setHint((data && data.error) || '保存偏好失败');
  }).catch(() => {});
}
function savePanePrefs(patch) {
  const ui = {}, playback = {}, window = {};
  Object.entries(patch).forEach(([k, v]) => {
    if (PLAYBACK_PREF_KEYS.has(k)) playback[k] = v;
    else if (WINDOW_PREF_KEYS.has(k)) window[k] = v;
    else ui[k] = v;
  });
  const calls = [];
  if (Object.keys(playback).length) calls.push(postPrefs('/api/playback-prefs', playback));
  if (Object.keys(window).length) calls.push(postPrefs('/api/window-prefs', window));
  if (Object.keys(ui).length) calls.push(postPrefs('/api/ui-prefs', ui));
  return Promise.all(calls);
}
function togglePane(which) {
  if (which === 'sidebar') state.sidebarCollapsed = !state.sidebarCollapsed;
  else if (which === 'group') state.groupCollapsed = !state.groupCollapsed;
  else if (which === 'detail') state.detailCollapsed = !state.detailCollapsed;
  else return;
  applyPaneLayout();
  // 详情栏宽度变化会改变歌曲表格的可用宽度 → 重新按占比分配列宽。
  if (which === 'detail') scheduleColumnWidths();
  const patch = which === 'sidebar'
    ? { sidebar_collapsed: state.sidebarCollapsed }
    : which === 'group'
      ? { group_collapsed: state.groupCollapsed }
      : { detail_collapsed: state.detailCollapsed };
  savePanePrefs(patch);
}
function setupPaneResize() {
  document.querySelectorAll('.pane-resizer').forEach((handle) => {
    handle.addEventListener('mousedown', (e) => {
      const which = handle.dataset.resize;
      const L = PANE_WIDTH_LIMITS[which];
      if (!L) return;
      const startX = e.clientX;
      const startW = which === 'sidebar' ? state.sidebarWidth : state.groupWidth;
      e.preventDefault();
      document.body.classList.add('pane-resizing');
      const cssVar = which === 'sidebar' ? '--sidebar-w' : '--group-w';
      const onMove = (ev) => {
        const w = Math.min(L.max, Math.max(L.min, startW + (ev.clientX - startX)));
        if (which === 'sidebar') state.sidebarWidth = w; else state.groupWidth = w;
        document.documentElement.style.setProperty(cssVar, Math.round(w) + 'px');
      };
      const onUp = () => {
        document.removeEventListener('mousemove', onMove);
        document.removeEventListener('mouseup', onUp);
        document.body.classList.remove('pane-resizing');
        const w = Math.round(which === 'sidebar' ? state.sidebarWidth : state.groupWidth);
        savePanePrefs(which === 'sidebar' ? { sidebar_width: w } : { group_width: w });
      };
      document.addEventListener('mousemove', onMove);
      document.addEventListener('mouseup', onUp);
    });
  });
  document.querySelectorAll('[data-toggle], [data-collapse]').forEach((b) => {
    b.addEventListener('click', (e) => {
      e.stopPropagation(); // 细条是贴在栏间间隙上的独立控件，点击不得触发该栏其它项的选中 / 切页
      togglePane(b.dataset.toggle || b.dataset.collapse);
    });
  });
}
// 音量：直接读写 audio.volume / audio.muted，持久化在 app 级 [ui]（不放 playstate.json）。
function applyVolume() {
  const v = Math.min(100, Math.max(0, Number(state.volume) || 0));
  audio.volume = v / 100;
  audio.muted = !!state.muted;
  const slider = $('volume');
  if (slider) slider.value = String(v);
  const btn = $('btn-mute');
  if (btn) {
    btn.textContent = (state.muted || v === 0) ? '🔇' : '🔊';
    btn.classList.toggle('is-active', !!state.muted);
  }
}
function setupVolume() {
  const slider = $('volume');
  if (slider) {
    // 拖动过程中防抖写盘：避免每个 input 事件都就地重写一次 kqk-app.toml。
    let volTimer = null;
    slider.addEventListener('input', () => {
      state.volume = Math.min(100, Math.max(0, Number(slider.value) || 0));
      if (state.volume > 0 && state.muted) state.muted = false;
      applyVolume();
      clearTimeout(volTimer);
      volTimer = setTimeout(() => {
        savePanePrefs({ volume: Math.round(state.volume), muted: state.muted });
      }, 150);
    });
  }
  const btn = $('btn-mute');
  if (btn) {
    btn.addEventListener('click', () => {
      state.muted = !state.muted;
      if (!state.muted && state.volume === 0) state.volume = 100;
      applyVolume();
      savePanePrefs({ muted: state.muted, volume: Math.round(state.volume) });
    });
  }
}
// 双击二级列表项 → 播放该列表整组全量（T1）。单击委托会先跑两次 selectXxx，
// 这里显式 await 最后一次，并发守卫只认最后一次请求，保证曲目加载完成后才有队列。
async function playGroupFromList(view, li) {
  const keyField = view === 'playlists' ? 'id' : 'name';
  const key = li.dataset[keyField];
  if (key === undefined || key === '') return;
  if (view === 'playlists') await selectPlaylist(Number(key));
  else if (view === 'albums') await selectAlbum(key);
  else if (view === 'artists') await selectArtist(key);
  const first = sourceTracks(view).find((x) => Number(x.local_id) > 0);
  if (!first) { setHint('该列表暂无可播放的歌曲'); return; }
  startPlayback(first, view, { fullGroup: true });
}
function bindGroupDoubleClick(listId, view) {
  const ul = $(listId);
  if (!ul) return;
  ul.addEventListener('dblclick', (e) => {
    const li = e.target.closest('li[data-id], li[data-name]');
    if (!li || li.classList.contains('pl-empty')) return;
    e.preventDefault();
    playGroupFromList(view, li);
  });
}

// ---------- FR-30：联网总闸 / 远程版本检测（本应用唯一联网点，默认全关） ----------
// 设计约束（用户 2026-11-08 拍板）：
//   · 仅向 api.github.com 发起一次简单 GET，取 Release 元数据；不读 assets[]、不下载、不安装、不写盘。
//   · 响应 256 KiB 流式上限 + 8s 硬超时 + 零自定义头；失败 / 404 / 限流 / 超限 / 非法 tag 一律静默。
//   · 频率 = 启动触发 + 间隔节流（daily 24h / weekly 7d / monthly 滚动 30 天），无后台定时器、无轮询。
//   · 总闸关闭时禁止一切非本机请求；子项值保留不清零，仅灰显禁用；关闭立即隐藏提示条与两个红点。
//   · 设置红点持久已读（seenFor），关于红点在「本地版本 < 远端版本」期间常驻，查看页面不熄灭。
//   · 缓存 / 已读标记 / 稍后状态放 localStorage，不污染 kqk-app.toml。
const UPDATE_SOURCES = {
  github: {
    label: 'GitHub',
    api: 'https://api.github.com/repos/KKitStudio/k-music/releases/latest',
    page: 'https://github.com/KKitStudio/k-music/releases',
  },
};
const UPDATE_INTERVALS = {
  daily: 24 * 3600 * 1000,
  weekly: 7 * 24 * 3600 * 1000,
  monthly: 30 * 24 * 3600 * 1000,
};
const UPDATE_CACHE_KEY = 'kqk_update_cache';
const UPDATE_MAX_BYTES = 256 * 1024;
const UPDATE_TIMEOUT_MS = 8000;
const UPDATE_TAG_RE = /^v?\d+\.\d+\.\d+$/;

function parseVersion(v) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(String(v || '').trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}
// 三段数字比较；任一非法返回 null（调用方一律按「无更新」处理）。
function compareVersions(a, b) {
  const x = parseVersion(a), y = parseVersion(b);
  if (!x || !y) return null;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i] ? 1 : -1;
  return 0;
}
function readUpdateCache() {
  try {
    const v = JSON.parse(localStorage.getItem(UPDATE_CACHE_KEY) || 'null');
    return (v && typeof v === 'object') ? v : {};
  } catch (e) { return {}; }
}
function writeUpdateCache(c) {
  try { localStorage.setItem(UPDATE_CACHE_KEY, JSON.stringify(c)); } catch (e) { /* 存储不可用则本次会话内仍生效 */ }
}
// 下载链接只接受 github.com 的 https URL，避免把远端返回的字符串直接塞进 href。
function safeReleaseUrl(u) {
  try {
    const url = new URL(String(u || ''));
    if (url.protocol === 'https:' &&
        (url.hostname === 'github.com' || url.hostname.endsWith('.github.com'))) return url.toString();
  } catch (e) { /* 非法 URL 回落 releases 页 */ }
  return UPDATE_SOURCES.github.page;
}
function setNetHint(msg) {
  const el = $('net-check-hint');
  if (el) el.textContent = msg || '';
}
function fmtCheckedAt(ms) {
  const n = Number(ms);
  if (!isFinite(n) || n <= 0) return '尚未检查';
  try { return new Date(n).toLocaleString(); } catch (e) { return '—'; }
}

// 流式读取并累计字节：超过 256 KiB 立即 cancel + abort，不读完、不解析。
async function readBoundedReleaseJson(res, controller) {
  if (res.body && typeof res.body.getReader === 'function') {
    const reader = res.body.getReader();
    const chunks = [];
    let total = 0;
    for (;;) {
      const part = await reader.read();
      if (part.done) break;
      const bytes = part.value;
      total += bytes.byteLength;
      if (total > UPDATE_MAX_BYTES) {
        try { await reader.cancel(); } catch (e) {}
        try { controller.abort(); } catch (e) {}
        return null;
      }
      chunks.push(bytes);
    }
    const buf = new Uint8Array(total);
    let off = 0;
    for (const c of chunks) { buf.set(c, off); off += c.byteLength; }
    try { return JSON.parse(new TextDecoder().decode(buf)); } catch (e) { return null; }
  }
  // 老 WebView2 无流式 body 时的降级：仍按 Blob.size 校验字节上限。
  try {
    const text = await res.text();
    if (new Blob([text]).size > UPDATE_MAX_BYTES) { controller.abort(); return null; }
    return JSON.parse(text);
  } catch (e) { return null; }
}

let updateCheckInFlight = false;
/**
 * @param {{manual?: boolean, force?: boolean}} opts
 *   manual = 设置页「立即检查」：无视频率强制执行一次，且仅此处反馈失败原因；
 *   force  = 忽略间隔节流（由 manual 使用）。
 */
async function checkForUpdate(opts) {
  const o = opts || {};
  // A3：总闸是硬门禁。关闭时绝不发出任何非本机请求，子项值仍保留。
  if (!state.networkEnabled || !state.updateCheckEnabled) {
    if (o.manual) setNetHint('请先开启联网总闸与「是否允许新版本检测」');
    return { ok: false, reason: 'disabled' };
  }
  const cache = readUpdateCache();
  const interval = UPDATE_INTERVALS[state.updateInterval] || UPDATE_INTERVALS.daily;
  const due = !cache.checkedAt || (Date.now() - Number(cache.checkedAt) >= interval);
  if (!o.force && !due) {
    // A6：频率未到不重新请求，但用缓存结果重现提示条与红点。
    applyUpdateUI();
    return { ok: true, reason: 'throttled' };
  }
  if (updateCheckInFlight) return { ok: false, reason: 'in-flight' };
  updateCheckInFlight = true;
  const src = UPDATE_SOURCES[state.updateSource] || UPDATE_SOURCES.github;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), UPDATE_TIMEOUT_MS);
  try {
    // 简单 GET + 零自定义头（浏览器自带 UA 已满足 GitHub API 要求；自定义头会触发 CORS 预检）。
    const res = await fetch(src.api, { signal: controller.signal });
    if (!res.ok) throw new Error('HTTP ' + res.status);
    const data = await readBoundedReleaseJson(res, controller);
    if (!data || typeof data !== 'object') throw new Error('bad payload');
    const tag = String(data.tag_name || '');
    if (!UPDATE_TAG_RE.test(tag)) throw new Error('bad tag');   // prerelease / 非三段一律放弃
    const next = Object.assign({}, cache, {
      latestVersion: tag,
      htmlUrl: safeReleaseUrl(data.html_url),
      checkedAt: Date.now(),
    });
    writeUpdateCache(next);
    // 检测成功才写一行本地 app.log（只含两个版本号）；失败路径不发请求、不落遥测。
    api('/api/update-check/log', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ local: state.app_version || '', remote: tag }),
    });
    const cmp = compareVersions(tag, state.app_version);
    if (o.manual) {
      setNetHint(cmp > 0 ? ('发现新版本 ' + tag + '（当前 ' + (state.app_version || '—') + '）')
                         : '已是最新版本 ' + (state.app_version || '—'));
    }
    applyUpdateUI();
    return { ok: true, tag: tag, cmp: cmp, reason: 'ok' };
  } catch (e) {
    // 自动检测失败一律静默（不弹错、不写日志）；仅「立即检查」在此处反馈原因。
    if (o.manual) setNetHint('检查未成功：网络不可达或暂时限流，可稍后重试');
    return { ok: false, reason: 'failed' };
  } finally {
    clearTimeout(timer);
    updateCheckInFlight = false;
  }
}

// 统一投射：提示条 + 两个红点 + 联网页状态。所有路径（启动 / 配置变更 / 检测完成）都汇聚这里。
function applyUpdateUI() {
  const cache = readUpdateCache();
  const local = String(state.app_version || '');
  const remote = String(cache.latestVersion || '');
  const cmp = (remote && local) ? compareVersions(remote, local) : null;
  const hasUpdate = cmp !== null && cmp > 0;
  const gate = !!(state.networkEnabled && state.updateCheckEnabled);

  // 提示条：总闸关闭立即隐藏；「稍后」只在本次会话隐藏，重启按 A6 重现。
  const bar = $('update-bar');
  const showBar = gate && hasUpdate && !state.updateBarDismissed;
  if (bar) bar.hidden = !showBar;
  if (showBar) {
    $('update-bar-text').textContent = '发现新版本 ' + remote + '（当前 ' + local + '）';
    $('update-bar-download').href = safeReleaseUrl(cache.htmlUrl);
  }
  // 设置红点：持久已读。仅检测到比 seenFor 更高的版本才重新亮。
  const dotSettings = $('update-dot-settings');
  if (dotSettings) {
    const seen = String(cache.seenFor || '');
    const newer = hasUpdate && (!seen || (compareVersions(remote, seen) || 0) > 0);
    dotSettings.hidden = !(gate && newer);
  }
  // 关于红点：有更新就一直在，只有本地版本 ≥ 远端版本才熄灭；打开 / 查看页面不熄灭。
  const dotAbout = $('update-dot-about');
  if (dotAbout) dotAbout.hidden = !(gate && hasUpdate);

  // 关于页「更新状态」区：仅开启新版本检测时可见（消除与联网页重复，未开启不打扰）。
  const aboutSec = $('about-update-section');
  if (aboutSec) aboutSec.hidden = !state.updateCheckEnabled;
  renderAboutUpdateUI(hasUpdate, gate);
  renderNetworkUI(hasUpdate, gate);
}

// 关于页「更新状态」区（仅开启检测时可见）：版本信息 + 有更新时红字提示 + 下载入口。
function renderAboutUpdateUI(hasUpdate, gate) {
  const cache = readUpdateCache();
  const set = (id, v) => { const el = $(id); if (el) el.textContent = v; };
  set('about-update-current', state.app_version ? ('v' + state.app_version) : '—');
  // 缓存的 latestVersion 取自 GitHub tag_name（形如 v0.9.0，已含前缀 v），直接显示即可，勿重复加 v。
  set('about-update-latest', cache.latestVersion ? cache.latestVersion : '—');
  set('about-update-checked-at', fmtCheckedAt(cache.checkedAt));
  let status = '检测未启用';
  if (gate && hasUpdate) status = '发现新版本';
  else if (gate && cache.latestVersion) status = '已是最新版本';
  else if (gate) status = '等待检查';
  set('about-update-status', status);

  const note = $('about-update-note');
  const dl = $('about-update-download');
  if (hasUpdate) {
    if (note) { note.textContent = '发现新版本 ' + cache.latestVersion + '（当前 v' + (state.app_version || '—') + '）'; note.hidden = false; }
    if (dl) { dl.href = safeReleaseUrl(cache.htmlUrl); dl.hidden = false; }
  } else {
    if (note) note.hidden = true;
    if (dl) dl.hidden = true;
  }
}

function renderNetworkUI(hasUpdate, gate) {
  const on = !!state.networkEnabled;
  const toggle = $('set-network-enabled');
  if (toggle) toggle.checked = on;
  const sub = $('set-update-check-enabled');
  if (sub) sub.checked = !!state.updateCheckEnabled;
  const srcSel = $('set-update-source');
  if (srcSel) srcSel.value = UPDATE_SOURCES[state.updateSource] ? state.updateSource : 'github';
  const intSel = $('set-update-interval');
  if (intSel) intSel.value = UPDATE_INTERVALS[state.updateInterval] ? state.updateInterval : 'daily';
  // 总闸关闭 → 子项整体灰显 + 逐控件 disabled（值保留不清零）；TODO 项恒 disabled。
  const box = $('set-network-subitems');
  if (box) box.classList.toggle('is-disabled', !on);
  ['set-update-check-enabled', 'set-update-source', 'set-update-interval', 'set-update-check-now'].forEach((id) => {
    const el = $(id);
    if (el) el.disabled = !on;
  });
  const todo = $('set-lyrics-remote-todo');
  if (todo) todo.disabled = true;
  // 状态信息已迁至「关于」页（仅开启检测时可见），此处仅同步控制项。
}

function saveNetworkPrefs(patch) {
  return api('/api/network-prefs', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(patch),
  }).then(({ data }) => {
    if (!data || !data.ok) setHint((data && data.error) || '保存联网偏好失败');
  }).catch(() => {});
}

// 外链统一走系统默认浏览器（Tauri 不处理 target=_blank）。
// Tauri v2 注意：窗口经 WebviewUrl::External 加载 http://127.0.0.1:<动态端口>，ACL 判为 Origin::Remote；
// capability 须用 remote.urls 放行该来源，否则 IPC 被 ACL 拒绝、回退 window.open 被 WebView 吞掉 → 点击无反应。
// 更正：withGlobalTauri 在 v2 下也会注入插件全局 API（__TAURI__.shell 并非「v1 恒空」）；旧写法与显式 IPC 落到同一条 ACL 检查。
// 正确路径是 IPC 命令 plugin:shell|open（载荷 { path }），由 lib.rs 已注册的
// tauri-plugin-shell 与 capabilities/update-download.json 的 shell:allow-open 授权给 main 窗口。
function openExternal(url) {
  if (!url) return;
  const fallback = () => {
    try { window.open(url, '_blank', 'noopener'); } catch (e) { /* 浏览器禁用弹窗/无 WebView */ }
  };
  let invoke = null;
  try {
    const t = window.__TAURI__;
    if (t && t.core && typeof t.core.invoke === 'function') invoke = t.core.invoke;
    else {
      // 窗口经 WebviewUrl::External 加载本机 axum 页面时，withGlobalTauri 注入的 __TAURI__
      // 可能不可用；Tauri v2 底层的 __TAURI_INTERNALS__.invoke 始终存在且命令签名一致。
      const internals = window.__TAURI_INTERNALS__;
      if (internals && typeof internals.invoke === 'function') invoke = internals.invoke;
    }
  } catch (e) { /* 落到浏览器回退 */ }
  if (!invoke) { fallback(); return; }
  try {
    const p = invoke('plugin:shell|open', { path: url });
    // 命令被拒 / 插件缺失 / capability 未生效时仍留一个出口，不让点击变成「无反应」。
    if (p && typeof p.catch === 'function') p.catch(fallback);
  } catch (e) { fallback(); }
}

function initUpdateEvents() {
  const master = $('set-network-enabled');
  if (master) master.addEventListener('change', (e) => {
    state.networkEnabled = !!e.target.checked;
    // A3：关闭总闸立即隐藏提示条与两个红点；开启总闸**不自动打开**检测子项。
    applyUpdateUI();
    saveNetworkPrefs({ network_enabled: state.networkEnabled });
    setNetHint('');
    // 总闸 + 子项都已开启时按正常节流规则尝试一次（不会绕过频率、不会在关闭态发请求）。
    if (state.networkEnabled && state.updateCheckEnabled) checkForUpdate({});
  });
  const sub = $('set-update-check-enabled');
  if (sub) sub.addEventListener('change', (e) => {
    state.updateCheckEnabled = !!e.target.checked;
    applyUpdateUI();
    saveNetworkPrefs({ update_check_enabled: state.updateCheckEnabled });
    if (state.networkEnabled && state.updateCheckEnabled) checkForUpdate({});
  });
  const src = $('set-update-source');
  if (src) src.addEventListener('change', (e) => {
    state.updateSource = UPDATE_SOURCES[e.target.value] ? e.target.value : 'github';
    renderNetworkUI();
    saveNetworkPrefs({ update_source: state.updateSource });
  });
  const interval = $('set-update-interval');
  if (interval) interval.addEventListener('change', (e) => {
    state.updateInterval = UPDATE_INTERVALS[e.target.value] ? e.target.value : 'daily';
    renderNetworkUI();
    saveNetworkPrefs({ update_check_interval: state.updateInterval });
  });
  const now = $('set-update-check-now');
  if (now) now.addEventListener('click', () => {
    // 手动：无视频率强制一次，失败原因只在此处反馈。
    setNetHint('正在检查…');
    checkForUpdate({ manual: true, force: true });
  });
  const later = $('update-bar-later');
  if (later) later.addEventListener('click', () => {
    state.updateBarDismissed = true;   // 仅本次会话隐藏，不写缓存 → 重启按 A6 重现
    applyUpdateUI();
  });
  // 外链：前往下载（提示条 / 关于页）+ 关于页项目主页，统一在系统浏览器打开。
  const bindOpen = (id) => {
    const el = $(id);
    if (el) el.addEventListener('click', (e) => {
      e.preventDefault();
      const href = el.getAttribute('href');
      if (href) openExternal(href);
    });
  };
  bindOpen('update-bar-download');
  bindOpen('about-update-download');
  bindOpen('about-homepage');
  // A5：点一次「设置」即把设置红点标记为持久已读；关于红点不受影响。
  const settingsBtn = $('btn-settings');
  if (settingsBtn) settingsBtn.addEventListener('click', () => {
    const cache = readUpdateCache();
    if (cache.latestVersion && cache.seenFor !== cache.latestVersion) {
      cache.seenFor = cache.latestVersion;
      writeUpdateCache(cache);
    }
    applyUpdateUI();
  });
}

let updateStartupChecked = false;
function startUpdateCheckOnce() {
  // A6：启动先按缓存重现提示条与红点（即使频率未到、不重新请求）。
  applyUpdateUI();
  if (updateStartupChecked) return;
  updateStartupChecked = true;
  // refreshStatus() 已读回 [ui]；只有总闸 + 子项都开才发请求，异步不阻塞首屏。
  if (state.networkEnabled && state.updateCheckEnabled) checkForUpdate({});
}

// ---------- 初始化 ----------
// 启动只调一次：refreshStatus() 在曲库已配置时会自行 loadTracks()，未配置则进入向导；
// 向导完成（finishInit）也会加载。无需在此重复 loadTracks。
// Phase A（2026-11-08，换界面友好化改造）：原「初始化区」整体包进 window.KQK.boot()，
// 不再在脚本加载时自动执行；改由薄壳 app.js 在 DOM 就绪后触发，使 core 逻辑与 skin 装配解耦。
window.KQK = window.KQK || {};
window.KQK.boot = function boot() {
  // 多语言骨架：DOM 就绪后一次性应用带 data-i18n 的静态节点。
  // 防御式调用：旧 dist/frontend 缺少 lang/ 脚本时优雅降级为中文原文，不阻断启动。
  if (typeof window.KQK.applyI18n === 'function') window.KQK.applyI18n();
  initAppearanceEvents();
  setupMediaSession();
  applyAppearance();
  loadAbout();
  // FR-30：联网页交互绑定先于 refreshStatus()，保证读回 [ui] 后所有投射函数都已就绪。
  initUpdateEvents();
  renderNetworkUI();
  // FR-24：侧栏 / 音量交互绑定与初始状态（refreshStatus 读回配置后会再投射一次）。
  setupPaneResize();
  setupVolume();
  bindGroupDoubleClick('pl-list', 'playlists');
  bindGroupDoubleClick('al-list', 'albums');
  bindGroupDoubleClick('ar-list', 'artists');
  applyPaneLayout();
  applyVolume();
  refreshStatus().then(() => {
    restorePlaystate();
    // FR-30：必须等 [ui] 四键读回后才决定是否联网；异步检测不阻塞首屏。
    // v0.12.11：只读曲源检查链已下线，启动不再发 /api/sources/check。
    startUpdateCheckOnce();
  }).catch(() => {});
  // FR-22 P1①：boot scan 在后台静默登记未决组，启动后补拉角标计数（**只更新角标，绝不自动弹窗**）。
  [1500, 5000, 12000].forEach((ms) => setTimeout(refreshDuplicateBadge, ms));
  setInterval(refreshDuplicateBadge, 30000);
};
