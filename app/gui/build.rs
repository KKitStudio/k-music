fn main() {
    tauri_build::build();

    // ---- 自动同步前端静态资源到发布目录 dist/frontend ----
    //
    // 背景：本项目用 `cargo build` 产出 exe，再由 exe 从磁盘提供前端（前端不嵌入二进制，
    // 见 core/src/serve.rs 的 resolve_frontend_dir：双击 dist/mt-gui.exe 时首个命中候选即 dist/frontend）。
    // `tauri.conf.json` 的 frontendDist 仅在 `cargo tauri build` 时才会被消费去嵌入前端，而发布走的是
    // `cargo build --release`，并不触发嵌入。因此前端必须作为静态文件物理存在于 exe 旁边。
    //
    // cargo 本身只编译 Rust，不会拷贝 HTML/JS/CSS，所以原本依赖 package_gui.bat 手动 xcopy。
    // 这里在编译期自动完成同步，使得 `cargo build --release -p mt-gui` 之后 dist/frontend 始终是最新，
    // 无需再单独跑 bat 来同步前端（bat 仅保留两份 exe + README 的拷贝，那是 cargo 无法自拷的部分）。
    //
    // ★ 保留运行时配置（2026-11-04）：本脚本**只同步 dist/frontend/**，
    //   绝不触碰 dist/ 下的运行时产物——kqk-app.toml（app 级配置，含用户测试时指定的当前曲库路径）、
    //   .guest/、library.db、app.log 等一律不删除、不覆盖。
    //   这样用户在 dist/ 内反复构建、反复测试时无需重新指定曲库。
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = std::path::Path::new(manifest_dir).join("frontend");
    let dst = std::path::Path::new(manifest_dir)
        .join("..") // app
        .join("..") // 工作区根
        .join("dist")
        .join("frontend");

    if src.exists() {
        if let Err(e) = copy_dir_all(&src, &dst) {
            // 同步失败不应阻断编译（例如 dist 正被占用），仅告警。
            eprintln!(
                "warning: 同步前端到 {} 失败: {}",
                dst.display(),
                e
            );
        }
    }

    // 仅当前端源码（含新增文件，目录监视会覆盖）变化时重跑本脚本，避免无谓重建。
    println!("cargo:rerun-if-changed=frontend");
}

/// 递归拷贝目录（覆盖式，保持目录结构）。
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
