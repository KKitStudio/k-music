/* Radix UI Colors 3.0.0 色阶与映射（全局共享，无框架依赖）。
   来源：@radix-ui/colors@3.0.0 官方 CSS。选中主色后套用同色系明/暗各 12 阶全套令牌。
   用法遵循 Radix 规范：1–2 底、3–5 淡底、6–8 描边、9 实心、10 hover、11–12 文字。
   index.html 首屏防闪烁脚本与 core.js::applyAppearance() 共用本文件，禁止各自复制色表。 */
(function (global) {
  'use strict';

  /* 中性灰三系：gray 走 NEUTRAL（下方），slate / sand 与彩色系同为 @radix-ui/colors@3.0.0 官方 12 阶。
     slate（冷银灰）/ sand（暖白）不参与彩色色相近似，仅由预设 9 阶实心色精确命中（见 map()）。 */
  var SCALES = {
    tomato: {
      light: ['#fffcfc','#fff8f7','#feebe7','#ffdcd3','#ffcdc2','#fdbdaf','#f5a898','#ec8e7b','#e54d2e','#dd4425','#d13415','#5c271f'],
      dark:  ['#181111','#1f1513','#391714','#4e1511','#5e1c16','#6e2920','#853a2d','#ac4d39','#e54d2e','#ec6142','#ff977d','#fbd3cb']
    },
    violet: {
      light: ['#fdfcfe','#faf8ff','#f4f0fe','#ebe4ff','#e1d9ff','#d4cafe','#c2b5f5','#aa99ec','#6e56cf','#654dc4','#6550b9','#2f265f'],
      dark:  ['#14121f','#1b1525','#291f43','#33255b','#3c2e69','#473876','#56468b','#6958ad','#6e56cf','#7d66d9','#baa7ff','#e2ddfe']
    },
    blue: {
      light: ['#fbfdff','#f4faff','#e6f4fe','#d5efff','#c2e5ff','#acd8fc','#8ec8f6','#5eb1ef','#0090ff','#0588f0','#0d74ce','#113264'],
      dark:  ['#0d1520','#111927','#0d2847','#003362','#004074','#104d87','#205d9e','#2870bd','#0090ff','#3b9eff','#70b8ff','#c2e6ff']
    },
    cyan: {
      light: ['#fafdfe','#f2fafb','#def7f9','#caf1f6','#b5e9f0','#9ddde7','#7dcedc','#3db9cf','#00a2c7','#0797b9','#107d98','#0d3c48'],
      dark:  ['#0b161a','#101b20','#082c36','#003848','#004558','#045468','#12677e','#11809c','#00a2c7','#23afd0','#4ccce6','#b6ecf7']
    },
    green: {
      light: ['#fbfefc','#f4fbf6','#e6f6eb','#d6f1df','#c4e8d1','#adddc0','#8eceaa','#5bb98b','#30a46c','#2b9a66','#218358','#193b2d'],
      dark:  ['#0e1512','#121b17','#132d21','#113b29','#174933','#20573e','#28684a','#2f7c57','#30a46c','#33b074','#3dd68c','#b1f1cb']
    },
    slate: {
      light: ['#fcfcfd','#f9f9fb','#f0f0f3','#e8e8ec','#e0e1e6','#d9d9e0','#cdced6','#b9bbc6','#8b8d98','#80838d','#60646c','#1c2024'],
      dark:  ['#111113','#18191b','#212225','#272a2d','#2e3135','#363a3f','#43484e','#5a6169','#696e77','#777b84','#b0b4ba','#edeef0']
    },
    sand: {
      light: ['#fdfdfc','#f9f9f8','#f1f0ef','#e9e8e6','#e2e1de','#dad9d6','#cfceca','#bcbbb5','#8d8d86','#82827c','#63635e','#21201c'],
      dark:  ['#111110','#191918','#222221','#2a2a28','#31312e','#3b3a37','#494844','#62605b','#6f6d66','#7c7b74','#b5b3ad','#eeeeec']
    }
  };
  var NEUTRAL = {
    light: ['#fcfcfc','#f9f9f9','#f0f0f0','#e8e8e8','#e0e0e0','#d9d9d9','#cecece','#bbbbbb','#8d8d8d','#838383','#646464','#202020'],
    dark:  ['#111111','#191919','#222222','#2a2a2a','#313131','#3a3a3a','#484848','#606060','#6e6e6e','#7b7b7b','#b4b4b4','#eeeeee']
  };

  function hexToRgb(hex) {
    var n = parseInt(hex.slice(1), 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  function rgbToHex(r, g, b) {
    function h(x) { return ('0' + Math.round(Math.max(0, Math.min(255, x))).toString(16)).slice(-2); }
    return '#' + h(r) + h(g) + h(b);
  }
  function rgbToHsl(r, g, b) {
    r /= 255; g /= 255; b /= 255;
    var max = Math.max(r, g, b), min = Math.min(r, g, b);
    var h = 0, s = 0, l = (max + min) / 2;
    if (max !== min) {
      var d = max - min;
      s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
      if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
      else if (max === g) h = (b - r) / d + 2;
      else h = (r - g) / d + 4;
      h /= 6;
    }
    return [h, s, l];
  }
  function isHex(v) { return typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v); }

  /** hex 精确命中任一色系任一步（明/暗 12 阶）则锁定该色系。
      近灰预设（gray/slate/sand 9 阶）饱和度极低，若走近似会被一律归到 gray，故先做精确匹配。 */
  function exactScale(hex) {
    if (!isHex(hex)) return null;
    var h = hex.toLowerCase(), names = ['gray'], k;
    for (k in SCALES) if (Object.prototype.hasOwnProperty.call(SCALES, k)) names.push(k);
    for (var n = 0; n < names.length; n++) {
      var tbl = names[n] === 'gray' ? NEUTRAL : SCALES[names[n]];
      for (var m = 0; m < 2; m++) {
        var steps = m === 0 ? tbl.light : tbl.dark;
        for (var i = 0; i < 12; i++) if (steps[i].toLowerCase() === h) return names[n];
      }
    }
    return null;
  }

  /** 按色相把任意 #RRGGBB 近似映射到最近的 Radix 色系；近灰阶映射到 gray 对应明度阶。 */
  function map(hex, theme) {
    if (!isHex(hex)) return null;
    var mode = theme === 'dark' ? 'dark' : 'light';
    var exact = exactScale(hex);
    if (exact) {
      return { scale: exact, shade: 9, hex: (exact === 'gray' ? NEUTRAL : SCALES[exact])[mode][8] };
    }
    var c = hexToRgb(hex), hsl = rgbToHsl(c[0], c[1], c[2]);
    var hue = hsl[0], sat = hsl[1], lig = hsl[2];
    if (sat < 0.12) {
      var steps = NEUTRAL[mode], shade = 9, best = Infinity;
      for (var i = 0; i < 12; i++) {
        var l = rgbToHsl.apply(null, hexToRgb(steps[i]))[2];
        var dd = Math.abs(l - lig);
        if (dd < best) { best = dd; shade = i + 1; }
      }
      return { scale: 'gray', shade: shade, hex: steps[shade - 1] };
    }
    var scale = 'tomato', bestHue = Infinity;
    for (var key in SCALES) {
      if (!Object.prototype.hasOwnProperty.call(SCALES, key)) continue;
      var ref = hexToRgb(SCALES[key][mode][8]);
      var hh = rgbToHsl(ref[0], ref[1], ref[2])[0];
      var dh = Math.abs(hue - hh);
      if (dh > 0.5) dh = 1 - dh;
      if (dh < bestHue) { bestHue = dh; scale = key; }
    }
    // Radix 规范：强调色取 9 阶实心色，其余阶由 accent() 成套派生。
    return { scale: scale, shade: 9, hex: SCALES[scale][mode][8] };
  }

  /** 生成一套完整 Radix 令牌（--radix-1..12 + 语义别名 + 各步数组），选中主色时一次性写入 :root。 */
  function accent(hex, theme) {
    var mapped = map(hex, theme) || { scale: 'tomato', shade: 9 };
    var mode = theme === 'dark' ? 'dark' : 'light';
    // gray 的表是 NEUTRAL 本身（无 'gray' 子键），彩色/中性灰新系在 SCALES 下按 scale 索引。
    var steps = mapped.scale === 'gray' ? NEUTRAL[mode] : SCALES[mapped.scale][mode];
    var vars = {};
    for (var i = 0; i < 12; i++) vars['--radix-' + (i + 1)] = steps[i];
    vars['--primary'] = steps[8];
    vars['--primary-dark'] = steps[9];
    vars['--primary-text'] = steps[10];
    vars['--primary-strong-text'] = steps[11];
    vars['--primary-subtle'] = steps[2];
    vars['--primary-border'] = steps[6];
    vars['--primary-glow'] = steps[7];
    return {
      scale: mapped.scale, shade: mapped.shade, steps: steps, vars: vars,
      primary: steps[8], dark: steps[9], text: steps[10], strongText: steps[11],
      subtle: steps[2], soft: steps[3], border: steps[6], glow: steps[7]
    };
  }

  global.Radix = {
    SCALES: SCALES, NEUTRAL: NEUTRAL, map: map, accent: accent,
    hexToRgb: hexToRgb, rgbToHex: rgbToHex, rgbToHsl: rgbToHsl, isHex: isHex
  };
})(window);
