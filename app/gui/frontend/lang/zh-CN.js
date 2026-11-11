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
    'settings.playback.auto_count': '当前：{count} 首'
  };
  var K = root.KQK = root.KQK || {};
  K.LANG = K.LANG || {};
  K.LANG['zh-CN'] = Object.freeze(dict);
  K.LOCALE = 'zh-CN';
})(window);
