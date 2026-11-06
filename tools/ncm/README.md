# ncmdump 备份（第三方 NCM 转换工具）

本目录为 musicMT 项目所用的第三方 NCM 解密/转换工具的本地备份。
**项目本身不实现 NCM 解密**（出于版权考虑），仅通过可配置的工具路径 + 命令模板调用本目录中的工具。

## 工具来源
- 仓库：https://github.com/taurusxin/ncmdump
- 许可证：MIT
- 语言：C++（CMake + taglib + cxxopts），跨平台（Windows / macOS / Linux）
- 说明：将网易云音乐 `.ncm` 加密文件还原为原始音频（mp3 / flac 等，解密后格式由源决定，不做转码）

## 备份内容
- `source/` —— 完整 git 克隆（含历史）。
  - HEAD 提交：`4ede4cee1bad35bbf28a7b5524feeca0422a4feb`（main，2026-08-20）
  - 同步方式：`git -C source pull` 可拉取上游更新
- `bin/ncmdump-1.5.1-windows-amd64.zip` —— Release 1.5.1 Windows CLI 压缩包（原始归档）
- `bin/libncmdump-1.5.1-windows-amd64.zip` —— Release 1.5.1 Windows 动态库压缩包（原始归档）
- `bin/cli/ncmdump.exe` —— 解压后的命令行工具（v1.5.1，已验证可运行）
- `bin/lib/libncmdump.dll` —— 解压后的动态库（可供 C#/Python/Java 等二次开发调用）

> 注：main 分支 HEAD 略新于 1.5.1 Release；如需与发布版本严格一致，可 `git -C source checkout 1.5.1`。

## 在 musicMT 中的调用方式
配置 `ncm.tool_path` 指向 `tools/ncm/bin/cli/ncmdump.exe`，系统按命令模板调用：

```
ncmdump.exe "{input}"            # 单文件/多文件
ncmdump.exe -d "{dir}" -r        # 递归批量
ncmdump.exe "{input}" -o "{out}" # 指定输出目录
```

支持的常用参数（详见 source/README.md）：
- `-d <dir>` 指定目录；`-r` 递归；`-o <dir>` 输出目录；`-m` 转换成功后删除源 ncm。

## 如何自行重新构建（可选）
- Windows：VS2022 + CMake + vcpkg（taglib 静态库），见 `source/README.md`。
- macOS / Linux：brew 安装 taglib 或源码编译，CMake 配置后 `cmake --build`。
- 也可直接 `brew install ncmdump`（macOS）或从各发行版仓库安装（Linux）。
