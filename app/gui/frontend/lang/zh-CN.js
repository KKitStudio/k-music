// K music 简体中文资源字典（当前唯一 locale：zh-CN）。
// 语义 key 采用点分命名空间；值里的 {name} 是插值占位（中文无复数，英文需要复数规则时另行扩展）。
// 本轮只收录示范模块「窗口与托盘」的静态文案与一处动态文案，
// 其余界面文案按模块分轮迁入；迁移期间未标注的中文原文保持不变。
(function (root) {
  'use strict';
  var dict = {
    'settings.tray.title': '窗口与托盘',
    'settings.tray.close': '关闭时最小化到托盘',
    'settings.tray.close_hint': '关闭主窗口（× / Alt+F4 / 任务栏右键关闭）时最小化到托盘而非退出。',
    'settings.tray.minimize': '最小化时隐藏到托盘',
    'settings.tray.minimize_hint': '最小化主窗口时隐藏到托盘（任务栏按钮消失），仅托盘可唤回。',
    'settings.playback.auto_count': '当前：{count} 首',
    // v0.12.2：顶栏入口改纯图标后，此键仍供 title / aria-label 悬浮提示使用（不可删，check_i18n.py 会校验未使用 key）。
    'sources.btn': '来源管理',
    'sources.title': '曲库来源管理',
    'sources.subtitle': '管理歌曲从哪些本地目录进入曲库',
    'sources.open': '打开来源管理',
    'sources.add': '＋ 新增来源',
    'sources.duplicates': '待处理重复',
    'sources.ncm': 'NCM 工具'
  };
  var K = root.KQK = root.KQK || {};
  K.LANG = K.LANG || {};
  K.LANG['zh-CN'] = Object.freeze(dict);
  K.LOCALE = 'zh-CN';
})(window);
