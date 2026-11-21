// K music 皮肤装配层（薄壳）。
// 本文件只负责「装配」：在 DOM 解析完成后触发 core.js 暴露的 window.KQK.boot()。
// 所有真实业务逻辑、API 调用、渲染、事件绑定都位于 core.js（逻辑内核，与 DOM 结构解耦）。
// 换皮肤时只需替换 index.html / app.js / styles.css 这一组「皮肤文件」，core.js 与后端不动。
// 背景：Phase A（2026-11-08，换界面友好化改造）引入 core/skin 拆分。
(function () {
  'use strict';
  function start() {
    var KQK = window.KQK || {};
    if (typeof KQK.boot === 'function') {
      KQK.boot();
    } else {
      console.error('[skin] KQK.boot 未找到：core.js 可能未加载或加载顺序错误');
    }
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
