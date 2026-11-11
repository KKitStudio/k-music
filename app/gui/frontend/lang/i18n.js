// K music 多语言骨架：t() 查表 / 插值 / 缺失回退 + applyI18n() 静态节点应用。
// 设计约束：无依赖、零网络请求、零定时器、零轮询；字典由 index.html 同步加载，
// 故 t() 在首帧即可用，不会有异步 fetch 造成的文案闪烁。
// 后续加语言包：扩 K.LANG 并按用户偏好设置 K.LOCALE，t() 契约不变。
(function (root) {
  'use strict';
  var K = root.KQK = root.KQK || {};
  var FALLBACK = ['zh-CN'];                 // 回退链：目标 locale → 默认中文
  var missing = Object.create(null);
  var warned = false;

  function dictionary(locale) {
    var all = K.LANG || {};
    return all[locale] || null;
  }

  function lookup(key) {
    var chain = [K.LOCALE || 'zh-CN'].concat(FALLBACK);
    for (var i = 0; i < chain.length; i++) {
      var d = dictionary(chain[i]);
      if (d && Object.prototype.hasOwnProperty.call(d, key)) return d[key];
    }
    return null;
  }

  // 命中则替换 {name} 占位；未命中绝不返回 key 本身，
  // 而是返回调用方给的 _fallback 或空串，并把缺失 key 记入 K.I18N_MISSING。
  function t(key, params) {
    var raw = lookup(key);
    if (raw === null) {
      missing[key] = true;
      return (params && typeof params._fallback === 'string') ? params._fallback : '';
    }
    if (!params) return raw;
    return String(raw).replace(/\{(\w+)\}/g, function (whole, name) {
      return Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : whole;
    });
  }

  // 只处理带 data-i18n 的节点；未命中时保留节点原有文本（渐进迁移期兜底）。
  function applyI18n(scope) {
    var nodes = (scope || document).querySelectorAll('[data-i18n]');
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i];
      var text = t(el.getAttribute('data-i18n'));
      if (text !== '') el.textContent = text;
    }
    flushI18nMissing();
  }

  // 汇总一次，避免运行期刷屏；K.I18N_MISSING 仍持续记录，供离线断言与诊断读取。
  function flushI18nMissing() {
    if (warned) return;
    var keys = Object.keys(missing);
    if (!keys.length) return;
    warned = true;
    if (root.console && typeof root.console.warn === 'function') {
      root.console.warn('[i18n] 缺失文案 key：' + keys.join(', '));
    }
  }

  K.t = t;
  K.applyI18n = applyI18n;
  K.flushI18nMissing = flushI18nMissing;
  K.I18N_MISSING = missing;
})(window);
