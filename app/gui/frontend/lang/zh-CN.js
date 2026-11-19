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
    // v0.12.2：顶栏入口改纯图标后，此键仍供 title / aria-label 悬浮提示使用。
    // v0.12.7：术语统一为「曲源」，旧词「曲库来源」「导入文件夹」退役。
    // v0.12.11：曲源总览改为设置子页；「检查 / 一键导入 / 持续监听」只读检查链下线，
    //           对应词条一并移除（check_i18n 要求字典与引用严格对齐）。
    'sources.btn': '曲源',
    'sources.title': '曲源',
    'sources.add': '添加曲源',
    'sources.add_title': '添加曲源',
    'sources.duplicates': '待处理重复',
    // v0.16.0：插件化后引导改为「安装插件」语义（原 sources.ncm / sources.ncm_not_ready 随曲源旧 NCM 块移除而删除）
    'sources.ncm_guide_title': '检测到网易云加密歌曲',
    'sources.ncm_guide_text': '该目录包含 .ncm 加密文件。未安装 NCM 插件时会跳过并留在源目录，只导入普通音频；安装插件后导入曲源即可处理。',
    'sources.ncm_setup_now': '现在安装',
    'sources.ncm_setup_later': '暂不安装',
    'sources.ncm_found_ready': '发现 .ncm 加密文件，导入时会转换',
    'sources.ncm_found_pending': '发现 .ncm 加密文件，未安装插件时会跳过并留在源目录',
    // v0.12.7：曲源总览分页 / rail 曲源页 / 进度面板
    'sources.tab': '曲源',
    'sources.overview': '曲源',
    'sources.refresh': '刷新',
    'sources.tracks_title': '曲源歌曲',
    'sources.update_all': '导入全部曲源',
    'sources.import_once': '导入此曲源',
    // v0.12.12：曲库文件夹内由扫描登记的歌曲来源明确，显示名由「未指定来源」改为「本地曲库」；
    // 并在曲源总览常显为内置特殊曲源（不可改名 / 不可删除 / 不参与更新）。内部 id 仍为 local_scan。
    'sources.local_library': '本地曲库',
    'sources.type_library': '内置曲源',
    'sources.builtin_meta': '内置，不可删除',
    'sources.builtin_tip': '曲库文件夹内由扫描登记的歌曲自动归入此曲源。它是内置项，不能改名、不能删除，也不参与「导入全部曲源」。',
    'sources.tip_readonly': '曲库是本地管理的曲库文件夹，曲源是外部上游目录；这里的重命名与删除不会改动曲库，也不会改动原始文件夹。',
    'sources.tip_rename': '仅改显示名称，不影响实际目录，也不改变已导入歌曲的归属。',
    'sources.delete_confirm': '确定删除「{name}」吗？只移除这条曲源记录，不会删除曲库或原始文件夹里的任何文件。',
    'sources.enabled': '启用',
    'sources.disabled': '停用',
    'sources.status_idle': '待导入',
    'sources.status_running': '导入中',
    'sources.status_done': '已完成',
    'sources.status_error': '导入失败',
    'sources.mode_copy': '复制',
    'sources.mode_move': '移动',
    'sources.type_netease': '网易云曲源',
    'sources.type_local': '本地曲源',
    'sources.delete': '删除',
    'sources.summary': '已完成 {done}/{total}，入库 {imported}，跳过 {skipped}',
    'progress.pause': '暂停',
    'progress.resume': '继续',
    'progress.stop': '停止',
    'progress.updating': '导入中',
    'progress.paused': '已暂停',
    'progress.stopped': '已停止',
    'progress.done': '已完成',
    'progress.conn_error': '连接异常，进度刷新暂不可用',
    'progress.stopped_keep': '已停止，保留已入库'
  };
  var K = root.KQK = root.KQK || {};
  K.LANG = K.LANG || {};
  K.LANG['zh-CN'] = Object.freeze(dict);
  K.LOCALE = 'zh-CN';
})(window);
