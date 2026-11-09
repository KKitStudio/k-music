// K music 前端（原生 JS + Material 风样式）。所有接口走相对路径（WebView 与后端同源）。
// 设计要点：网易云服务端登录/下载已移除（2026-10-29）；本地导入源仅「从本地网易云曲库更新」按钮。

const $ = (id) => document.getElementById(id);
const audio = $('audio');

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
  ncm_tool: '',
  pending_ncm: 0,
  // FR-22：未决重复组数（W15 只统计当前曲库、按组数）与挂起目录 / 独立日志路径。
  pendingDuplicates: 0,
  duplicatesDir: '',
  duplicatesLogPath: '',
  duplicateGroups: [],
  tracks: [],
  // 全部曲库视图的筛选状态（会话内有效，不持久化；音质 quality / 仅看收藏 favorite）。
  tracksQuery: '',
  trackFilters: { quality: '', favorite: false },
  current: null,
  // 分视图选中态：四个页面各自的「曲级选中」互不干扰。
  // value = { track: 列表中的曲目摘要, trx: /api/track 返回的完整详情 } 或 null。
  // 旧实现只有一个全局 state.current，切页后详情栏会残留上一页的数据。
  viewSelection: { library: null, playlists: null, albums: null, artists: null },
  // 每页当前是否有可展示的详情内容（渲染时更新，供可见性仲裁使用）。
  // 无内容 → 该页的详情区（右侧栏 / 内联条）整体隐藏，不渲染占位内容。
  detailShown: { library: false, playlists: false, albums: false, artists: false },
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
};

// 歌曲列表列定义（key → 表头文案 + 单元格渲染）。顺序即默认显示顺序。
const COLUMN_DEFS = {
  name: { label: '曲名', render: (t) => esc(t.name) },
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
  try {
    const res = await fetch(path, opts);
    let data = null;
    try { data = await res.json(); } catch (e) { /* 空体 */ }
    return { ok: res.ok, data };
  } catch (e) {
    return { ok: false, data: null };
  }
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
};

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
  skipped: '已跳过',
  pending: '待转换',
  failed: '失败',
};

function showProgressPanel(title) {
  state.progressTitle = title || '正在处理…';
  const panel = $('progress-panel');
  if (!panel) return;
  panel.hidden = false;
  progressState.visible = true;
  progressState.logSeq = 0;
  $('pp-title').textContent = state.progressTitle;
  $('pp-count').textContent = '';
  $('pp-current').textContent = '';
  $('pp-summary').textContent = '';
  const log = $('pp-log');
  if (log) log.innerHTML = '';
  const fill = $('pp-bar-fill');
  if (fill) fill.style.width = '0%';
}

// 启动进度轮询（每 400ms 拉一次）。幂等：重复调用不会起多个定时器。
function startProgressPolling() {
  stopProgressPolling();
  progressState.timer = setInterval(pollProgress, 400);
}

function stopProgressPolling() {
  if (progressState.timer) {
    clearInterval(progressState.timer);
    progressState.timer = null;
  }
}

async function pollProgress() {
  const { data } = await api('/api/import-progress');
  if (!data) return;
  renderProgress(data);
}

function renderProgress(p) {
  const panel = $('progress-panel');
  if (!panel) return;
  if (!progressState.visible) return;

  // 标题：后端给出本次任务来源（如「本地网易云曲库」）。
  const title = $('pp-title');
  if (title && p.source) title.textContent = p.source + ' · 更新中';

  // 计数：已处理 / 总数。
  const count = $('pp-count');
  if (count) {
    count.textContent = p.total > 0 ? `${p.processed} / ${p.total}` : `${p.processed}`;
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

  // 底部汇总计数。
  const sum = $('pp-summary');
  if (sum) {
    const parts = [`入库 ${p.imported}`, `跳过 ${p.skipped}`];
    if (p.pending) parts.push(`待转换 ${p.pending}`);
    if (p.failed) parts.push(`失败 ${p.failed}`);
    sum.textContent = parts.join(' · ');
  }
}

function hideProgressPanel() {
  stopProgressPolling();
  const panel = $('progress-panel');
  if (panel) panel.hidden = true;
  progressState.visible = false;
}

// 面板关闭按钮：用户查看完明细后手动收起（不影响正在进行的导入）。
const ppClose = $('pp-close');
if (ppClose) ppClose.addEventListener('click', hideProgressPanel);

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

function setHint(msg) { $('hint').textContent = msg || ''; }

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
    state.ncm_tool = data.ncm_tool || '';
    state.pending_ncm = data.pending_ncm || 0;
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
  // 曲库状态 + NCM 工具状态（网易云登录态已移除，不再展示）。
  if (!state.library_configured) {
    st.textContent = '未指定曲库';
  } else if (!state.ncm_ready) {
    st.textContent = '曲库就绪（未配置 NCM 工具）';
  } else {
    st.textContent = '曲库就绪';
  }
}

// ---------- 布局风格（横向 / 纵向）----------
// 横向(1) = 曲目详情显示在窗口右侧的独立详情栏；
// 纵向(2) = 详情内联到歌曲列表上方，把 300px 宽度让给列表（长曲名/艺人/专辑不被截断）。
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
  const c = LAYOUT_VIEWS[view];
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
  const inline = (state.layoutStyles[view] || 1) === 2;
  // FR-25：纵向布局时右侧详情栏隐藏，列表内图标按钮需贴列表右缘。
  document.body.classList.toggle('detail-inline', inline);
  // 右侧独立详情栏：纵向时隐藏（宽度让给列表）。
  // 横向且已收起时窄条必须常驻（图标按钮就在这里，是唯一的展开入口），
  // 只有「展开态 + 该页无任何内容」才整体隐藏，避免留下一条空栏。
  $('detail-pane').hidden = inline || (!state.detailShown[view] && !state.detailCollapsed);
  // 内联详情条：仅当前页 + 纵向 +（有内容 或 处于收起态）时显示。
  VIEW_KEYS.forEach((v) => {
    const el = $(LAYOUT_VIEWS[v].wrap);
    if (el) el.hidden = !(inline && v === view && (state.detailShown[v] || state.detailCollapsed));
  });
  // FR-25 第五轮：详情细条始终跟着「当前真正可见的详情区」走（横向=右侧栏，纵向=内联水平条）。
  const dh = $('handle-detail');
  if (dh) {
    const host = inline ? $(LAYOUT_VIEWS[view].wrap) : $('detail-pane');
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
  set('cfg-version', state.config_version ? 'v' + state.config_version : '—');
  set('cfg-min-version', state.config_min_version ? 'v' + state.config_min_version : '—');
  set('cfg-app-version', state.app_version ? 'v' + state.app_version : '—');
  set('cfg-library-version', state.library_config_version ? 'v' + state.library_config_version : '—');
  const label = COMPAT_LABELS[state.config_compat] || state.config_compat || '—';
  const libLabel = COMPAT_LABELS[state.library_config_compat];
  set('cfg-compat', libLabel ? label + ' / 曲库：' + libLabel : label);
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
async function loadTracks(q) {
  state.tracksQuery = q || '';
  const params = new URLSearchParams();
  if (state.tracksQuery) params.set('q', state.tracksQuery);
  if (state.trackFilters.quality) params.set('quality', state.trackFilters.quality);
  if (state.trackFilters.favorite) params.set('favorite', '1');
  const { data } = await api('/api/tracks?' + params.toString());
  state.tracks = (data && data.tracks) || [];
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

// 全部曲库视图筛选条（音质下拉 + 仅看收藏）。
(function initTrackFilters() {
  const fq = $('filter-quality');
  const ff = $('filter-fav');
  if (fq) {
    fq.addEventListener('change', () => {
      state.trackFilters.quality = fq.value;
      loadTracks(state.tracksQuery);
    });
  }
  if (ff) {
    ff.addEventListener('click', () => {
      state.trackFilters.favorite = !state.trackFilters.favorite;
      ff.classList.toggle('active', state.trackFilters.favorite);
      loadTracks(state.tracksQuery);
    });
  }
})();

function renderTracks() {
  const head = $('track-head');
  const body = $('track-body');
  // 表头：最左侧固定 # 序号列 + 可见业务列（按 state.columns 顺序）。
  // 业务列带 data-col-key，供列宽拖拽与占比分配使用。
  const cols = visibleTrackCols();
  head.innerHTML = '<th class="idx-col">#</th>' + trackHeadHtml(cols);
  body.innerHTML = '';
  state.tracks.forEach((t, i) => {
    const tr = document.createElement('tr');
    tr.dataset.id = t.local_id;
    tr.innerHTML = idxCellHtml(t, i) + trackCellsHtml(t, cols);
    tr.addEventListener('click', () => selectTrack(t, tr));
    tr.addEventListener('dblclick', () => { selectTrack(t, tr); playTrack(t); });
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
    // 任一列不得窄于像素下限（受单元格内边距限制）；差额回推给对方。
    if (bPx < MIN_COL_PX) { bPx = MIN_COL_PX; aPx = totalPx - bPx; }
    if (aPx < MIN_COL_PX) { aPx = MIN_COL_PX; bPx = totalPx - aPx; }
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
  if (!state.tracks.length) {
    state.current = null;
    state.viewSelection.library = null;
    clearDetail();
    return;
  }
  // 只认「全部曲库」页自己的选中态，避免其它页面的选中串进来。
  const prev = state.viewSelection.library;
  let sel = null;
  if (prev && prev.track) {
    sel = state.tracks.find((t) => t.local_id === prev.track.local_id) || null;
  }
  if (!sel) sel = state.tracks[0];
  const tr = $('track-body').querySelector('tr[data-id="' + sel.local_id + '"]');
  if (!tr) return;
  // 仅当切换了选中对象时才重新拉取详情，避免搜索过滤时反复请求。
  if (!prev || !prev.track || prev.track.local_id !== sel.local_id) {
    selectTrack(sel, tr);
  } else {
    tr.classList.add('selected');
  }
}

// 曲库无曲目 / 未选中时清空「全部曲库」页的详情（并同步右侧详情栏）。
function clearDetail() {
  state.viewSelection.library = null;
  renderViewDetail('library');
  if (state.activeTab === 'library') renderActiveDetail();
}

// 刷新按钮旁的统计：当前列表（已按搜索过滤）的歌曲数量。
function updateTrackCount() {
  $('track-count').textContent = `共 ${state.tracks.length} 首`;
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

function sourceTracks(view) {
  if (view === 'library') return state.tracks || [];
  if (view === 'playlists') return state.playlistTracks || [];
  if (view === 'albums') return state.albumTracks || [];
  if (view === 'artists') return state.artistTracks || [];
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
  // FR-23：智能三视图（全部曲库 / 专辑 / 歌手）统一「智能播放列表 · …」前缀；
  // 有名播放列表仍属手工维护列表，标题不变。
  if (view === 'playlists' && state.currentPlaylist) {
    return '播放列表 · ' + state.currentPlaylist.name;
  }
  if (view === 'albums') {
    return '智能播放列表 · 专辑：' + (state.currentAlbum ? state.currentAlbum.name : '未知专辑');
  }
  if (view === 'artists') {
    return '智能播放列表 · 艺人：' + (state.currentArtist ? state.currentArtist.name : '未知艺人');
  }
  return named ? '播放列表' : '智能播放列表 · 全部曲库';
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

function playCurrent() {
  const pb = state.playback;
  const id = pb.queue[pb.index];
  if (!id) { setHint('队列已空'); return; }
  const m = pb.meta[id] || {};
  audio.src = '/api/audio/' + id;
  audio.play().catch(() => {});
  $('now-playing').textContent = (m.name || ('#' + id)) + (m.artist ? ' — ' + m.artist : '');
  $('btn-play').textContent = '⏸';
  loadLyrics(id);
  renderQueueUI();
  savePlaystate();
}

// 上一首 / 下一首 / 自动推进。auto=true 表示由 audio.ended 触发。
function stepQueue(delta, auto) {
  const pb = state.playback;
  if (!pb.queue.length) { setHint('当前没有播放队列'); return; }
  let next = pb.index + delta;
  if (next < 0 || next >= pb.queue.length) {
    if (!pb.repeat) {
      if (auto) {
        $('btn-play').textContent = '▶';
        setHint('播放结束');
      } else {
        setHint(delta > 0 ? '已是队列最后一首' : '已是队列第一首');
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
    setHint('已建立自动播放列表，并把该曲加入其中');
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
  setHint('已插到下一首播放，并同步加入当前播放列表');
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
  $('now-playing').textContent = '未在播放';
  $('btn-play').textContent = '▶';
  renderQueueUI();
  api('/api/playstate', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ clear: true }),
  });
  setHint('播放队列已清空');
}

// 队列面板：超过窗口阈值时只渲染当前位置附近的一段，避免上千行 DOM 卡顿。
function renderQueueUI() {
  const pb = state.playback;
  const total = pb.queue.length;
  $('queue-title').textContent = total ? (pb.title + ' · ' + total + ' 首') : '';
  $('queue-drawer-title').textContent = pb.title || '播放队列';
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
  pb.title = s.title || '播放队列';
  pb.named = !!s.named;
  pb.n = s.n == null ? null : Number(s.n);
  pb.shuffle = !!s.shuffle;
  pb.repeat = !!s.repeat;
  state.playback = pb;
  const id = pb.queue[pb.index];
  const m = pb.meta[id] || {};
  $('now-playing').textContent = (m.name || ('#' + id)) + '（已暂停 · 点播放继续）';
  renderQueueUI();
}

// ---------- 播放列表通用对话框（替代 prompt / confirm，避免原生对话框抢焦点）----------
let dialogResolve = null;
function showDialog(opts) {
  return new Promise((resolve) => {
    dialogResolve = resolve;
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
$('pl-dialog-cancel').addEventListener('click', () => closeDialog(null));
$('pl-dialog-input').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') closeDialog(e.target.value);
  if (e.key === 'Escape') closeDialog(null);
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
    setHint(`「${t.name || '未命名'}」已加入播放列表`);
    return;
  }

  if (act === 'remove') { await removeFromPlaylist(id); return; }

  if (act === 'reveal') {
    const r = await ctxPost('/api/tracks/reveal', { local_id: id });
    setHint((r.data && r.data.ok) ? '已在资源管理器中定位文件' : ((r.data && r.data.error) || '定位失败'));
    return;
  }

  if (act === 'copy') {
    const text = (t.name || '未命名') + (t.artist ? ' — ' + t.artist : '') + (t.album ? ' · ' + t.album : '');
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
      setHint('已复制歌曲信息');
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
      '\n音频文件会移入曲库回收站 .mt/trash/（可手动取回），' +
      '索引以及所有播放列表中的引用会一并清除。此操作不会放入系统回收站。',
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
  setHint(`已删除「${t.name || '未命名'}」` +
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
  ['tracks', 'playlistTracks', 'albumTracks', 'artistTracks'].forEach((k) => {
    state[k] = (state[k] || []).filter((x) => Number(x.local_id) !== id);
  });
  ['library', 'playlists', 'albums', 'artists'].forEach((v) => {
    const sel = state.viewSelection[v];
    if (sel && sel.track && Number(sel.track.local_id) === id) state.viewSelection[v] = null;
  });
  if (state.current && Number(state.current.local_id) === id) state.current = null;
  if (view === 'library') await loadTracks($('search').value);
  else if (view === 'playlists' && state.currentPlaylist) await loadPlaylistTracks(state.currentPlaylist.id);
  else if (view === 'albums' && state.currentAlbum) await loadAlbumTracks(state.currentAlbum.name);
  else if (view === 'artists' && state.currentArtist) await loadArtistTracks(state.currentArtist.name);
  await loadPlaylists();
  if (state.activeTab === 'albums') await loadAlbums();
  if (state.activeTab === 'artists') await loadArtists();
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
  const srcEl = $('lyrics-source');
  if (srcEl) srcEl.textContent = '';
  if (!localId) { renderLyrics(); return; }
  const { data } = await api('/api/lyrics/' + localId);
  if (!data || !data.ok) { renderLyrics(); return; }
  const text = data.text || '';
  if (!text.trim()) { renderLyrics(); return; }
  state.lyrics = parseLrc(text);
  state.lyricsSource = data.source || '';
  if (srcEl) {
    srcEl.textContent =
      data.source === 'lrc' ? 'LRC 文件' : (data.source === 'embedded' ? '内嵌歌词' : '');
  }
  renderLyrics();
  syncLyrics();
}

// 渲染歌词面板。
function renderLyrics() {
  const body = $('lyrics-body');
  if (!body) return;
  const L = state.lyrics;
  if (!L) {
    body.innerHTML = '<div class="placeholder">暂无歌词</div>';
    return;
  }
  if (!L.timed || !L.lines.length) {
    // 纯文本歌词：整段展示（去掉元信息行）。
    const plain = L.plain.split(/\r?\n/)
      .filter((l) => l.trim() && !/^\[[a-zA-Z]+:.*\]$/.test(l.trim()))
      .map((l) => `<div class="lyric-line">${esc(l.trim())}</div>`)
      .join('');
    body.innerHTML = plain || '<div class="placeholder">暂无歌词</div>';
    return;
  }
  body.innerHTML = L.lines
    .map((l, i) => `<div class="lyric-line" data-i="${i}" data-t="${l.t}">${esc(l.text) || '&nbsp;'}</div>`)
    .join('');
  // 点击某行跳转到对应时间点。
  body.querySelectorAll('.lyric-line').forEach((el) => {
    el.addEventListener('click', () => {
      const t = parseFloat(el.dataset.t);
      if (!isNaN(t) && audio.duration) audio.currentTime = t;
    });
  });
}

// 按播放进度同步高亮行并居中滚动。
function syncLyrics() {
  const L = state.lyrics;
  if (!L || !L.timed || !L.lines.length) return;
  const now = audio.currentTime;
  let idx = -1;
  for (let i = 0; i < L.lines.length; i++) {
    if (L.lines[i].t <= now + 0.05) idx = i;
    else break;
  }
  if (idx === state.lyricsIndex) return;
  state.lyricsIndex = idx;
  const body = $('lyrics-body');
  if (!body) return;
  body.querySelectorAll('.lyric-line').forEach((el) => el.classList.remove('current'));
  if (idx < 0) return;
  const el = body.querySelector(`.lyric-line[data-i="${idx}"]`);
  if (el) {
    el.classList.add('current');
    body.scrollTop = el.offsetTop - body.clientHeight / 2 + el.offsetHeight / 2;
  }
}

// 歌词面板开关。
$('btn-lyrics').addEventListener('click', () => {
  const drawer = $('lyrics-drawer');
  drawer.hidden = !drawer.hidden;
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
// 文件失效 / 无法解码：跳过该 id，不中断整个队列（FR-18 已拍板）。
let errorSkips = 0;
audio.addEventListener('error', () => {
  // 无头断言环境里 /api/audio 不可达，禁用自动跳过以免队列被连续推进（仅供测试置位）。
  if (window.__KQK_DISABLE_AUDIO_ERROR_SKIP__) return;
  const pb = state.playback;
  if (!audio.src || !pb.queue.length) return;
  errorSkips += 1;
  if (errorSkips > pb.queue.length) {
    setHint('队列中多首歌曲已失效，播放停止');
    errorSkips = 0;
    return;
  }
  setHint('曲目文件失效或无法播放，已跳过');
  const next = pb.index + 1;
  if (next >= pb.queue.length) {
    if (pb.repeat) { pb.index = 0; playCurrent(); }
    else { $('btn-play').textContent = '▶'; }
  } else {
    pb.index = next;
    playCurrent();
  }
});
audio.addEventListener('playing', () => { errorSkips = 0; });
$('seek').addEventListener('input', () => {
  if (audio.duration) audio.currentTime = audio.duration * ($('seek').value / 1000);
});

$('btn-prev').addEventListener('click', () => stepQueue(-1, false));
$('btn-next').addEventListener('click', () => stepQueue(1, false));
$('btn-shuffle').addEventListener('click', () => {
  const pb = state.playback;
  pb.shuffle = !pb.shuffle;
  if (pb.shuffle) applyShuffle(); else restoreOriginalOrder();
  renderQueueUI();
  savePlaystate();
  setHint(pb.shuffle ? '随机播放：已打乱当前队列并固定顺序' : '顺序播放：已恢复界面顺序');
});
$('btn-repeat').addEventListener('click', () => {
  const pb = state.playback;
  pb.repeat = !pb.repeat;
  renderQueueUI();
  savePlaystate();
  setHint(pb.repeat ? '列表循环已开启' : '列表循环已关闭（顺序播放到末尾停止）');
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

// ---------- 工具栏 ----------
$('btn-scan').addEventListener('click', async () => {
  setHint('扫描中…');
  const { data } = await api('/api/scan', { method: 'POST' });
  setHint(data && data.ok ? `扫描完成：新增 ${data.inserted}，更新 ${data.updated}，移除 ${data.removed}${data.held ? `，重复挂起 ${data.held}` : ''}` : '扫描失败');
  // 手动扫描是新决策点：即使之前忽略过，发现散落 .ncm 仍再次询问。
  maybeShowNcmOrganize((data && data.ncm_found) || [], true);
  // FR-22 P1①：手动「扫描曲库」才打开批量对比窗；boot scan 只静默登记角标。
  if (data && data.ok && (data.pending_duplicates || 0) > 0) {
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
  const btn = $('btn-duplicates');
  if (!btn) return;
  $('dup-badge').textContent = String(count);
  btn.hidden = count === 0;
  btn.title = count ? `查看 ${count} 组待处理重复文件（只统计当前曲库）` : '暂无待处理重复文件';
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
    ['来源', m.origin ? (DUP_ORIGIN_LABELS[m.origin] || m.origin) : ''],
  ].filter(([, v]) => v !== '' && v != null);
  if (!rows.length) return '';
  return '<div class="dup-option-attrs">' +
    rows.map(([k, v]) => `<span>${k}</span><b>${esc(v)}</b>`).join('') + '</div>';
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

function renderDuplicateGroups(list) {
  const box = $('dup-groups');
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
        g, c.id, `新文件 ${ci + 1} · ${label}`, c,
        `/api/duplicates/preview/${encodeURIComponent(c.id)}`, selected === c.id
      ));
    });
    const div = document.createElement('div');
    div.className = 'dup-group';
    div.innerHTML =
      `<div class="dup-group-title">${gi + 1}. ${esc(g.title || '未知曲目')}</div>` +
      `<div class="dup-group-meta">${esc(g.artist || '')} · ${esc((g.format || '').toUpperCase())} · ${options.length} 个版本 · ${esc(DUP_ORIGIN_LABELS[g.origin] || g.origin || '')}</div>` +
      (g.last_choice ? `<div class="dup-last">${esc(g.last_choice)}（已为你预选，可改）</div>` : '') +
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
    ? `挂起文件：${state.duplicatesDir}（稍后处理不会移动文件；换曲库时随曲库保留）` : '';
  renderDuplicateGroups(list);
  $('duplicates-modal').hidden = false;
}

function closeDuplicatesDialog() {
  stopDuplicatePreview();
  $('duplicates-modal').hidden = true;
}

async function submitDuplicateDecisions() {
  const inputs = Array.from(document.querySelectorAll('#dup-groups input[type="radio"]:checked'));
  const decisions = inputs.map((el) => ({ group_id: el.dataset.groupId, choice: el.value }))
    .filter((d) => d.group_id && d.choice);
  if (!decisions.length) { setHint('请先为每组选择要保留的版本'); return; }
  const btn = $('dup-apply');
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
  else setHint(`已处理 ${data.applied} 组重复文件`);
  updateDupBadge(data.pending_duplicates);
  if ((Number(data.pending_duplicates) || 0) > 0) {
    // 仍有未决组（决策失败 / 别的批次）：保留窗口并重新拉取。
    await openDuplicatesDialog();
  } else {
    closeDuplicatesDialog();
  }
  await refreshDuplicateBadge();
  loadTracks($('search').value);
}

$('btn-duplicates').addEventListener('click', () => openDuplicatesDialog());
$('dup-later').addEventListener('click', () => {
  // W4：稍后处理不提交决策，挂起文件保持留在 .mt/pending-duplicates/。
  closeDuplicatesDialog();
  setHint(state.pendingDuplicates
    ? `已保留 ${state.pendingDuplicates} 组待处理重复文件，可随时点顶栏角标继续` : '已关闭对比窗口');
});
$('dup-apply').addEventListener('click', () => submitDuplicateDecisions());
$('dup-groups').addEventListener('change', (e) => {
  const input = e.target;
  if (!input.matches || !input.matches('input[type="radio"]')) return;
  const wrap = input.closest('.dup-options');
  if (!wrap) return;
  wrap.querySelectorAll('.dup-option').forEach((el) =>
    el.classList.toggle('is-selected', el.contains(input)));
});
$('dup-groups').addEventListener('click', (e) => {
  const btn = e.target.closest && e.target.closest('[data-preview-url]');
  if (btn) { e.preventDefault(); previewDuplicate(btn.dataset.previewUrl, btn); }
});
$('dup-preview-audio').addEventListener('ended', () => {
  if (dupPreviewBtn) dupPreviewBtn.textContent = '▶ 试听';
  dupPreviewBtn = null;
});
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && !$('duplicates-modal').hidden) closeDuplicatesDialog();
});
$('set-dup-clear-history').addEventListener('click', async () => {
  const yes = await showDialog({
    title: '清除同步/导入历史',
    text: '将清空本曲库 .mt/sync-import.jsonl 的同步与导入流水，重复对比窗口不再给出「上次你选择了…」预选。挂起中的重复组不受影响。',
    okText: '清除',
  });
  if (!yes) return;
  const { data } = await api('/api/duplicates/history/clear', { method: 'POST' });
  setHint(data && data.ok ? '同步/导入历史已清除' : ((data && data.error) || '清除历史失败'));
});

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
  stopProgressPolling();
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
  setHint(`散落 .ncm 处理完成：${parts.join('，')}`);
  await refreshStatus();
  loadTracks($('search').value);
});
$('btn-refresh').addEventListener('click', () => loadTracks($('search').value));
$('search').addEventListener('input', (e) => loadTracks(e.target.value));

// ---------- 左侧 tab 选择栏（全部曲库 / 播放列表 / 专辑 / 歌手）----------
function switchTab(tab) {
  closeTrackContextMenu();
  state.activeTab = tab;
  $('tab-library').classList.toggle('active', tab === 'library');
  $('tab-playlists').classList.toggle('active', tab === 'playlists');
  $('tab-albums').classList.toggle('active', tab === 'albums');
  $('tab-artists').classList.toggle('active', tab === 'artists');
  $('library-pane').hidden = (tab !== 'library');
  $('playlists-pane').hidden = (tab !== 'playlists');
  $('albums-pane').hidden = (tab !== 'albums');
  $('artists-pane').hidden = (tab !== 'artists');
  // 按当前页面的布局风格决定：右侧独立详情栏（横向）还是列表上方内联详情条（纵向）。
  applyLayoutStyle();
  // 右侧详情栏是四个页面共用的单一 DOM：切页后必须按新页面的选中态重算，
  // 否则会残留上一页（例如曲库页）选中的歌曲数据。
  renderActiveDetail();
  // 表格高亮同步到新页面的选中项。
  syncSelectionHighlight(tab);
  // 目标页的内联详情条也按该页自己的选中态重算（与右侧详情栏保持同一来源）。
  renderViewDetail(tab);
  // 切到对应面板时载入列表。播放列表页额外重拉当前列表曲目：
  // 加入 / 移除 / 删除歌曲可能发生在别的视图，右侧表格不能沿用上次加载的快照。
  if (tab === 'playlists') {
    loadPlaylists();
    if (state.currentPlaylist) loadPlaylistTracks(state.currentPlaylist.id);
  }
  if (tab === 'albums') loadAlbums();
  if (tab === 'artists') loadArtists();
}
$('tab-library').addEventListener('click', () => switchTab('library'));
$('tab-playlists').addEventListener('click', () => switchTab('playlists'));
$('tab-albums').addEventListener('click', () => switchTab('albums'));
$('tab-artists').addEventListener('click', () => switchTab('artists'));

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
    li.innerHTML =
      `<div class="gcard-art">${cover}<span class="gcard-fallback" aria-hidden="true">♫</span></div>` +
      (name !== null ? `<span class="gcard-name">${esc(name)}</span>` : '') +
      (count !== null ? `<span class="gcard-count">${esc(count)} 首</span>` : '');
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
  setHint('已创建播放列表：' + name);
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
      '<div class="placeholder">选择一个播放列表以查看内容，或点击「新建」创建播放列表。</div>';
    return;
  }
  if (!state.playlistTracks.length) {
    box.innerHTML =
      '<div class="placeholder">该播放列表暂无歌曲。在「全部曲库」选中一首歌，再点上方「＋ 加入选中歌曲」。</div>';
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
    tr.addEventListener('dblclick', () => { selectTrack(t, tr); playTrack(t); });
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
  setHint('播放列表已重命名：' + name);
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
  setHint('播放列表已导出');
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
  setHint('已导入播放列表：' + data.name);
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
  setHint(`已加入「${state.currentPlaylist.name}」：${libTrack.name}`);
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
  $('pl-rename').hidden = true;
  $('pl-export').hidden = true;
  $('pl-delete').hidden = true;
  await loadPlaylists();
  renderViewDetail('playlists');
  if (state.activeTab === 'playlists') renderActiveDetail();
  $('pl-content').innerHTML =
    '<div class="placeholder">选择一个播放列表以查看内容，或点击「新建」创建播放列表。</div>';
  setHint('已删除播放列表');
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
  // 右键菜单要知道触发来源：al-content = 专辑页，ar-content = 歌手页。
  const ctxView = boxId === 'al-content' ? 'albums' : 'artists';
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
    tr.addEventListener('dblclick', () => { selectTrack(t, tr); playTrack(t); });
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
    renderGroupCards('al-list', state.albums, '暂无智能播放列表 · 专辑', selected);
  } else {
    // 默认「列表视图」= 原有行为，renderGroupList 一字不改，保证既有回归断言全绿。
    if (list) list.classList.remove('grid-view');
    renderGroupList('al-list', state.albums, '暂无智能播放列表 · 专辑', 'name', selected);
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
    if (!img || img.tagName !== 'IMG' || !img.closest('.gcard-art')) return;
    // 只隐藏 img，露出常驻的 .gcard-fallback；不删节点，避免破图与 DOM 结构抖动。
    img.style.display = 'none';
  }, true);
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
  renderBrowseTracks('al-content', state.albumTracks, '该智能播放列表暂无歌曲。');
}

// ---------- 歌手 ----------
async function loadArtists() {
  const { data } = await api('/api/artists');
  state.artists = (data && data.artists) || [];
  renderArtists();
}

function renderArtists() {
  renderGroupList(
    'ar-list',
    state.artists,
    '暂无智能播放列表 · 歌手',
    'name',
    state.currentArtist ? state.currentArtist.name : null,
  );
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
  renderBrowseTracks('ar-content', state.artistTracks, '该智能播放列表暂无歌曲。');
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
  setHint('曲库名称已保存：' + state.library_name);
  updateLibraryName();
  await refreshStatus();
});
// 本地网易云曲库目录：浏览 / 保存 / 一键更新
$('set-netease-browse').addEventListener('click', async () => {
  const { data } = await api('/api/pick-folder', { method: 'POST' });
  if (data && data.ok && !data.canceled && data.path) {
    $('set-netease-input').value = data.path;
    $('set-netease-status').textContent = '已选择：' + data.path;
  } else if (!data || !data.ok) {
    setHint((data && data.error) || '打开文件夹选择失败');
  }
});
// 保存「本地网易云音乐曲库目录」（app 级配置 → kqk-app.toml）。成功返回 true。
async function saveNeteaseDir(path) {
  const p = (path || '').trim();
  if (!p) { setHint('请先选择或输入本地网易云曲库目录'); return false; }
  const { data } = await api('/api/netease/library-dir', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path: p }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '保存失败'); return false; }
  state.netease_library_dir = data.library_dir || p;
  return true;
}
$('set-netease-apply').addEventListener('click', async () => {
  setHint('保存目录中…');
  if (await saveNeteaseDir($('set-netease-input').value.trim())) {
    setHint('本地网易云曲库目录已保存');
    await refreshStatus();
  }
});
// 设置面板的「更新」：输入框已填路径则先保存，再执行更新；否则走「未设置 → 弹选择框」流程。
$('set-netease-update').addEventListener('click', async () => {
  const typed = $('set-netease-input').value.trim();
  if (typed && typed !== state.netease_library_dir) {
    if (!await saveNeteaseDir(typed)) return;
  }
  await runImportNetease();
});
$('set-ncm-pick').addEventListener('click', pickNcmTool.bind(null, 'set-ncm-input'));
$('set-ncm-apply').addEventListener('click', applyNcmTool.bind(null, 'set-ncm-input', 'set-ncm-copy'));
$('set-apply-scheme').addEventListener('click', applyScheme);

// ---------- 设置面板：左侧标签导航 ↔ 右侧分栏内容 ----------
// 纯前端 class 切换，不改动任何设置项的控件与业务逻辑。
function activateSettingsTab(key) {
  const tabs = document.querySelectorAll('#settings-modal .settings-tab');
  const panes = document.querySelectorAll('#settings-modal .settings-pane');
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
document.querySelectorAll('#settings-modal .settings-tab').forEach((tab) => {
  tab.setAttribute('role', 'tab');
  tab.addEventListener('click', () => activateSettingsTab(tab.dataset.tab));
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
  $('set-library-name').textContent = state.library_name || '（未命名）';
  $('set-name-input').value = state.library_name || '';
  document.querySelectorAll('#set-scheme-options input').forEach((r) => {
    r.checked = (r.value === state.file_scheme);
  });
  syncSchemeOptionState();
  $('set-ncm-status').textContent = state.ncm_ready
    ? ('已启用：' + state.ncm_tool)
    : '未配置（无 NCM 工具时，.ncm 文件会进入 download/pendingNcm 暂存）';
  $('set-netease-dir').textContent = state.netease_library_dir || '（未设置）';
  $('set-netease-input').value = state.netease_library_dir || '';
  renderColConfig();
  renderLayoutStyleUI();
  renderColWidthUI();
  renderAppearanceUI();
  renderAbout();
  renderConfigStatus();
  $('set-auto-count').value = String(state.autoPlaylistCount);
  $('set-auto-count-hint').textContent = '当前：' + state.autoPlaylistCount + ' 首';
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
  setHint(LAYOUT_VIEWS[view].label + ' 已切换为' + LAYOUT_STYLE_LABELS[style]);
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
  setHint('已恢复默认列宽比例');
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
  setHint('歌曲列表显示设置已保存');
});

// 设置 → 播放：n 只写 app 级 [ui] 段，下次生成队列才生效（当前队列的 n 已快照）。
$('set-apply-playback').addEventListener('click', async () => {
  const n = clampAutoCount($('set-auto-count').value);
  $('set-auto-count').value = String(n);
  const { data } = await api('/api/ui-prefs', {
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
  setHint('自动播放列表上限已保存：' + state.autoPlaylistCount + ' 首');
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
    setHint(`曲库目录已更新：${state.library_dir}（新增 ${scan.data.inserted}，更新 ${scan.data.updated}，移除 ${scan.data.removed}）`);
    maybeShowNcmOrganize(scan.data.ncm_found || [], true);
    if ((scan.data.pending_duplicates || 0) > 0) await openDuplicatesDialog();
  } else {
    setHint(`目录已保存，但扫描失败：${(scan.data && scan.data.error) || '请稍后重试'}`);
  }
  await refreshStatus();
}

$('btn-import').addEventListener('click', () => {
  $('import-input').value = '';
  $('import-modal').hidden = false;
});
$('btn-import-netease').addEventListener('click', runImportNetease);

// 从本地网易云曲库目录增量合并（本地操作，不联网/不登录，单方向只加不删）。
// 未设置「本地网易云音乐曲库目录」时：先弹出目录选择框，指定成功后立即执行更新。
async function runImportNetease() {
  if (!state.netease_library_dir) {
    setHint('尚未设置「本地网易云音乐曲库目录」，请选择该目录…');
    const picked = await api('/api/pick-folder', { method: 'POST' });
    const pd = picked && picked.data;
    if (!pd || !pd.ok) { setHint((pd && pd.error) || '打开文件夹选择失败'); return; }
    if (pd.canceled || !pd.path) { setHint('已取消指定本地网易云音乐曲库目录，未执行更新'); return; }
    if (!await saveNeteaseDir(pd.path)) return;
    setHint('已指定本地网易云音乐曲库目录，正在更新（只加不删）…');
  } else {
    setHint('从本地网易云曲库更新中（只加不删）…');
  }
  // 打开进度面板并开始轮询，让用户看到逐首的处理明细。
  showProgressPanel('本地网易云曲库');
  startProgressPolling();
  const { data } = await api('/api/import-netease', { method: 'POST' });
  stopProgressPolling();
  if (data && data.ok) {
    const parts = [`入库 ${data.imported}`];
    if (data.skipped) parts.push(`跳过 ${data.skipped}`);
    if (data.pending) parts.push(`待转换 ${data.pending}`);
    if (data.failed) parts.push(`失败 ${data.failed}`);
    if (data.held) parts.push(`重复挂起 ${data.held}`);
    setHint(`从本地网易云曲库更新完成：${parts.join('，')}`);
    await refreshDuplicateBadge();
    if (data.held) await openDuplicatesDialog();
    loadTracks($('search').value);
  } else {
    setHint((data && data.error) || '从本地网易云曲库更新失败');
  }
  // 面板保留在屏上供用户查看明细，由「×」手动关闭。
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
  stopProgressPolling();
  setHint(data && data.ok
    ? `导入完成：新增 ${data.imported}，跳过 ${data.skipped}${data.pending ? `，待转换 ${data.pending}` : ''}${data.held ? `，重复挂起 ${data.held}` : ''}，失败 ${data.failed}`
    : (data && data.error) || '导入失败');
  // FR-22 P4：批量结束后一起弹对比选择窗（全部未决组一次展示）。
  await refreshDuplicateBadge();
  if (data && data.ok && data.held) await openDuplicatesDialog();
  loadTracks($('search').value);
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
let wizStep = 1;
function showInitWizard() {
  $('init-wizard').hidden = false;
  wizStep = 1;
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
  $('wiz-ncm-input').value = '';
  renderWizStep();
}
function renderWizStep() {
  document.querySelectorAll('.wizard-step').forEach((el) => {
    el.hidden = (Number(el.dataset.step) !== wizStep);
  });
  $('wiz-prev').hidden = (wizStep <= 1);
  $('wiz-next').hidden = (wizStep >= 4);
  $('wiz-finish').hidden = (wizStep < 4);
  // 第 4 步（本地网易云曲库目录）可选，可跳过。
  $('wiz-skip').hidden = (wizStep !== 4);
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
    $('wiz-netease-status').textContent = '已选择：' + data.path;
  } else if (!data || !data.ok) {
    setHint((data && data.error) || '打开文件夹选择失败');
  }
});
$('wiz-next').addEventListener('click', () => {
  if (wizStep === 1 && !$('wiz-library-input').value.trim()) {
    setHint('请先指定曲库目录');
    return;
  }
  if (wizStep < 4) { wizStep++; renderWizStep(); }
});
$('wiz-prev').addEventListener('click', () => { if (wizStep > 1) { wizStep--; renderWizStep(); } });
$('wiz-skip').addEventListener('click', finishInit);
$('wiz-ncm-pick').addEventListener('click', () => pickNcmTool('wiz-ncm-input'));
$('wiz-finish').addEventListener('click', finishInit);

async function finishInit() {
  const path = $('wiz-library-input').value.trim();
  if (!path) { setHint('请先指定曲库目录'); return; }
  const scheme = document.querySelector('input[name="wiz-scheme"]:checked').value;
  const library_name = $('wiz-name-input').value.trim();
  setHint('初始化曲库中…');
  const r = await api('/api/library-directory', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path, file_scheme: scheme, library_name }),
  });
  if (!r.data || !r.data.ok) { setHint((r.data && r.data.error) || '初始化失败'); return; }
  state.library_dir = r.data.library_dir;
  state.library_name = r.data.library_name || library_name;
  // 可选：指定 NCM 工具
  const ncmPath = $('wiz-ncm-input').value.trim();
  if (ncmPath) await applyNcmTool('wiz-ncm-input', 'wiz-ncm-copy');
  // 可选：本地网易云曲库目录（第 4 步，可跳过）
  const neteasePath = $('wiz-netease-input').value.trim();
  if (neteasePath) {
    const ne = await api('/api/netease/library-dir', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: neteasePath }),
    });
    if (ne.data && ne.data.ok) state.netease_library_dir = ne.data.library_dir || neteasePath;
  }
  $('init-wizard').hidden = true;
  setHint('曲库已初始化，正在扫描…');
  await refreshStatus();
  const scan = await api('/api/scan', { method: 'POST' });
  if (scan.data && scan.data.ok) {
    setHint(`初始化完成：${state.library_dir}（新增 ${scan.data.inserted}，更新 ${scan.data.updated}，移除 ${scan.data.removed}）`);
    maybeShowNcmOrganize(scan.data.ncm_found || [], true);
  }
  loadTracks($('search').value);
}

// ---------- NCM 工具指定 ----------
async function pickNcmTool(inputId) {
  const { data } = await api('/api/pick-ncm-tool', { method: 'POST' });
  if (data && data.ok && !data.canceled && data.path) $(inputId).value = data.path;
  else if (!data || !data.ok) setHint((data && data.error) || '打开文件选择失败');
}
async function applyNcmTool(inputId, copyId) {
  const path = $(inputId).value.trim();
  if (!path) { setHint('请先选择 NCM 工具文件'); return; }
  const copy = $(copyId).checked;
  setHint('应用 NCM 工具中…');
  // 指定工具后会自动回补 pendingNcm 积压（可能几十上百首）→ 打开进度面板。
  showProgressPanel('NCM 积压转换');
  startProgressPolling();
  const { data } = await api('/api/ncm-tool', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path, copy_to_library: copy }),
  });
  stopProgressPolling();
  if (!data || !data.ok) { setHint((data && data.error) || '应用失败'); return; }
  state.ncm_ready = !!data.tool_ready;
  state.ncm_tool = data.tool_path || '';
  if (data.backlog) {
    const b = data.backlog;
    setHint(`NCM 工具已应用：${data.tool_path}；pendingNcm 积压转换：入库 ${b.imported}，待转换 ${b.pending}，失败 ${b.failed}`);
  } else {
    setHint('NCM 工具已应用：' + data.tool_path);
  }
}

// ---------- 文件结构方案切换（重排）----------
async function applyScheme() {
  const sel = document.querySelector('input[name="set-scheme"]:checked');
  if (!sel) return;
  if (sel.value === state.file_scheme) { setHint('文件结构方案未变化'); return; }
  if (!confirm('切换方案将按新结构整体移动 / 重命名曲库文件，目录结构将大幅变化。确定继续？')) return;
  setHint('正在按新方案重组曲库…');
  const { data } = await api('/api/reorganize', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ scheme: sel.value }),
  });
  if (!data || !data.ok) { setHint((data && data.error) || '重组失败'); return; }
  state.file_scheme = sel.value;
  setHint(`重组完成：已移动 ${data.moved} 个文件`);
  loadTracks($('search').value);
}

// ================= 外观 / 主题 / 换肤 =================
// 主题方案以 **app 级配置文件 kqk-app.toml 的 [ui] 段为准**（与布局风格 / 列宽同源），
// localStorage 只作为首屏即时缓存 + 自定义皮肤图片本体（base64，体积大，不写 TOML）。
// 每次改动防抖 POST /api/ui-prefs 落盘；启动时由 /api/status 读回并覆盖本地缓存。
// FR-29：主题色功能已移除，--primary / --primary-dark 恒由 styles.css 的明暗主题变量给出；
// 配置里的 accent / skin_accent 是「兼容遗留」字段，前端一律不读、不写、不派生（勿复活）。
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

// 预设皮肤：CSS 渐变（无需图片文件），只作为底层背景，不再派生任何主题色。
const SKIN_PRESETS = [
  { id: 'aurora', name: '极光', gradient: 'linear-gradient(135deg,#0f2027,#203a43,#2c5364)' },
  { id: 'sakura', name: '樱粉', gradient: 'linear-gradient(135deg,#ffecd2,#fcb69f)' },
  { id: 'mint', name: '薄荷', gradient: 'linear-gradient(135deg,#a1ffce,#faffd1)' },
  { id: 'purple', name: '紫霞', gradient: 'linear-gradient(135deg,#667eea,#764ba2)' },
  { id: 'ocean', name: '深海', gradient: 'linear-gradient(135deg,#2193b0,#6dd5ed)' },
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
  // 默认跟随系统：未保存过偏好时不做任何显式主题假设。
  // FR-29：两个数值与皮肤同属外观方案，随配置一起持久化；皮肤对象不再含 accent。
  return {
    theme: 'system',
    lyricsOpacity: LYRICS_OPACITY_DEFAULT,
    skinBlur: SKIN_BLUR_DEFAULT,
    skin: { enabled: false, type: null, src: null, gradient: null, name: null },
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
    skin_type: type,
    skin_preset: (type === 'preset' && preset) ? preset.id : '',
    // FR-29：两个可调数值随外观一起落盘。后端 appearance 段是部分更新语义，
    // 不再提交废弃的 accent / skin_accent（旧值留在配置里但没有任何消费者）。
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
      name: preset.name, src: null,
    };
  } else if (cfg.skin_type === 'custom' && local.skin && local.skin.type === 'custom' && local.skin.src) {
    skin = Object.assign({}, local.skin, { enabled: true });
  }
  // 旧版 localStorage 里的驼峰键名也兼容；配置注入用下划线键名。
  return {
    theme,
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
  // 布局皮肤：即使外观尚未「被用户触碰」也照常应用（boot 时已按注入值预设，这里与状态对齐）。
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

// FR-29：主题色功能已移除，原先的颜色转换工具与 extractDominantColor()（64×64 canvas 取色）
// 一并删除 —— 选皮肤不再解码图片取色，只把压缩后的图片作为底层背景。勿复活。

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
// FR-29：主题色固定，每次外观变化后上报的是明暗主题对应的品牌蓝常量（与 styles.css 一致）。
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

  // FR-29：主题色固定由 styles.css 的 [data-theme] 变量给出（浅 #4f7cff / 暗 #6e92ff）。
  // 显式清掉可能残留的行内覆盖（旧版页面 / 旧缓存注入），否则 CSS 变量永远被行内值压住。
  root.style.removeProperty('--primary');
  root.style.removeProperty('--primary-dark');

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

  // 标题栏与顶栏同色：所有外观变化路径都汇聚到这里，天然保持同步。
  syncNativeTitlebar(eff === 'dark' ? '#6e92ff' : '#4F7CFF', eff);
}

function setSkin(skin) {
  const a = loadAppearance();
  a.skin = skin
    ? Object.assign({ enabled: true }, skin)
    : { enabled: false, type: null, src: null, gradient: null, name: null };
  saveAppearance(a);
  applyAppearance();
  renderAppearanceUI();
}

// ---------- 渲染外观设置 UI ----------
function renderAppearanceUI() {
  if (!$('ap-skins')) return;
  const a = loadAppearance();

  // 主题模式分段控件（按设置值高亮，system 为中间项）
  document.querySelectorAll('#ap-theme-seg .seg-btn').forEach((b) => {
    b.classList.toggle('active', b.dataset.theme === a.theme);
  });
  // 布局皮肤下拉：回显当前值（layoutSkin 在 boot / syncAppearanceFromStatus 时已设置）。
  const skinSel = document.getElementById('ap-layout-skin');
  if (skinSel) skinSel.value = layoutSkin;
  // 列表密度下拉：回显当前值。
  const densitySel = document.getElementById('ap-list-density');
  if (densitySel) densitySel.value = listDensity;
  const hint = $('ap-theme-hint');
  if (hint) {
    hint.textContent = a.theme === 'system'
      ? `跟随系统：当前系统为${systemDark() ? '深色' : '浅色'}，系统主题变化会自动同步`
      : (a.theme === 'dark' ? '已固定为深色主题' : '已固定为浅色主题');
  }

  // FR-29：主题色色板 / 自定义取色器已整体删除，勿复活。
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
      el.addEventListener('click', () => setSkin({ type: 'preset', gradient: p.gradient, name: p.name }));
      sk.appendChild(el);
    });
  }
  document.querySelectorAll('#ap-skins .skin-thumb').forEach((el) => {
    const on = a.skin && a.skin.enabled;
    if (el.classList.contains('none')) el.classList.toggle('active', !on);
    else el.classList.toggle('active', on && a.skin.type === 'preset' && a.skin.gradient === el.style.background);
  });

  // 当前皮肤说明：FR-29 起只说明底层背景，不再提「自动提取主题色」。
  const cur = $('ap-skin-current');
  if (cur) {
    if (a.skin && a.skin.enabled) {
      const label = a.skin.type === 'preset' ? ('预设「' + (a.skin.name || '') + '」') : '自定义图片';
      cur.textContent = `当前皮肤：${label}（仅作底层背景，模糊 ${clampSkinBlur(a.skinBlur)}px，不改变主题色）`;
    } else {
      cur.textContent = '当前：无皮肤（模糊滑块在选择皮肤后生效）';
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
}

// ---------- 布局皮肤切换（classic / modern）----------
// 共享 core.js 引擎，仅换 CSS：boot 时已按 layout_skin 预设 <link> 与 html.skin-modern 类，
// 这里提供运行时切换 + 持久化。皮肤是「整体配色 / 视觉」层，与底层背景皮肤（skin_type）正交。
let layoutSkin = 'classic';
function applyLayoutSkin(name, persist) {
  if (name !== 'modern' && name !== 'classic') name = 'classic';
  layoutSkin = name;
  const link = document.getElementById('skin-css');
  if (link) link.href = 'skins/' + name + '.css';
  document.documentElement.classList.toggle('skin-modern', name === 'modern');
  const sel = document.getElementById('ap-layout-skin');
  if (sel) sel.value = name;
  if (persist) {
    fetch('/api/ui-prefs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ layout_skin: name }),
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
  const sel = document.getElementById('ap-list-density');
  if (sel) sel.value = name;
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

  // 布局皮肤切换：实时换 CSS 并持久化到 [ui].layout_skin（经 /api/ui-prefs 就地更新）。
  const layoutSkinSel = document.getElementById('ap-layout-skin');
  if (layoutSkinSel) layoutSkinSel.addEventListener('change', () => applyLayoutSkin(layoutSkinSel.value, true));
  // 列表密度切换：实时改行高 / 内边距并持久化到 [ui].list_density。
  const densitySel = document.getElementById('ap-list-density');
  if (densitySel) densitySel.addEventListener('change', () => applyListDensity(densitySel.value, true));

  const pick = $('ap-skin-pick');
  if (pick) pick.addEventListener('click', () => { const f = $('ap-skin-file'); if (f) f.click(); });
  const file = $('ap-skin-file');
  if (file) file.addEventListener('change', (e) => {
    const f = e.target.files && e.target.files[0];
    if (!f) return;
    // FR-29：只把压缩后的图片设为底层背景，不再二次解码取色（少一次 Image 解码 + canvas 取样）。
    fileToSkinDataURL(f, 1600, (dataUrl) => {
      if (!dataUrl) { setHint('图片读取失败'); return; }
      setSkin({ type: 'custom', src: dataUrl, name: '自定义' });
      setHint('已应用自定义皮肤（仅作底层背景，不改变主题色）');
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
    sh.textContent = state.sidebarCollapsed ? '›' : '‹';
    setPaneTip(sh, state.sidebarCollapsed ? '展开一级菜单' : '收起一级菜单');
  }
  document.querySelectorAll('.pane-handle.group-handle').forEach((b) => {
    b.textContent = state.groupCollapsed ? '›' : '‹';
    setPaneTip(b, state.groupCollapsed ? '展开二级列表' : '收起二级列表');
  });
  const dh = $('handle-detail');
  if (dh) {
    const inline = document.body.classList.contains('detail-inline');
    dh.textContent = inline
      ? (state.detailCollapsed ? '⌄' : '⌃')
      : (state.detailCollapsed ? '‹' : '›');
    setPaneTip(dh, (state.detailCollapsed ? '展开' : '收起') + (inline ? '歌曲详情' : '歌曲信息栏'));
  }
}
// 统一把内存状态投射到 DOM：宽度用 CSS 变量（body class 决定收起态），按钮常驻可展开。
function applyPaneLayout() {
  document.documentElement.style.setProperty('--sidebar-w', state.sidebarWidth + 'px');
  document.documentElement.style.setProperty('--group-w', state.groupWidth + 'px');
  document.body.classList.toggle('sidebar-collapsed', !!state.sidebarCollapsed);
  document.body.classList.toggle('group-collapsed', !!state.groupCollapsed);
  // FR-25：歌曲信息栏收缩态（含封面），与两侧栏共用同一套持久化语义。
  document.body.classList.toggle('detail-collapsed', !!state.detailCollapsed);
  updatePaneHandleArrows();
}
// 所有 [ui] 写入都走 /api/ui-prefs（后端 save_ui_in_place，绝不 save_app 整份重写）。
function savePanePrefs(patch) {
  api('/api/ui-prefs', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(patch),
  }).then(({ data }) => {
    if (!data || !data.ok) setHint((data && data.error) || '保存界面偏好失败');
  }).catch(() => {});
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
    if (o.manual) setNetHint('检查未成功：网络不可达或暂时限流，稍后可重试（不影响使用）');
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

  renderNetworkUI(hasUpdate, gate);
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

  const cache = readUpdateCache();
  const set = (id, v) => { const el = $(id); if (el) el.textContent = v; };
  set('net-current-version', state.app_version ? ('v' + state.app_version) : '—');
  set('net-latest-version', cache.latestVersion || '—');
  set('net-checked-at', fmtCheckedAt(cache.checkedAt));
  const g = (gate === undefined) ? !!(state.networkEnabled && state.updateCheckEnabled) : gate;
  // 事件回调可能不带参调用；此时自行从缓存推导，避免状态行短暂显示错误。
  const local = String(state.app_version || '');
  const remote = String(cache.latestVersion || '');
  const derived = (remote && local) ? (compareVersions(remote, local) > 0) : false;
  const up = (hasUpdate === undefined) ? derived : hasUpdate;
  let status = '联网已关闭';
  if (state.networkEnabled && !state.updateCheckEnabled) status = '检测未启用';
  else if (g && up) status = '发现新版本（可自行下载）';
  else if (g && cache.latestVersion) status = '已是最新版本';
  else if (g) status = '等待检查';
  set('net-status', status);
}

function saveNetworkPrefs(patch) {
  return api('/api/ui-prefs', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(patch),
  }).then(({ data }) => {
    if (!data || !data.ok) setHint((data && data.error) || '保存联网偏好失败');
  }).catch(() => {});
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
  initAppearanceEvents();
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
    startUpdateCheckOnce();
  }).catch(() => {});
  // FR-22 P1①：boot scan 在后台静默登记未决组，启动后补拉角标计数（**只更新角标，绝不自动弹窗**）。
  [1500, 5000, 12000].forEach((ms) => setTimeout(refreshDuplicateBadge, ms));
  setInterval(refreshDuplicateBadge, 30000);
};
