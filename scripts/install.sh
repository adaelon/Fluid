#!/bin/sh
# Fluid installer — downloads the prebuilt `fluid` binary for your platform and
# puts it on your PATH. One-line install:
#   curl -fsSL https://github.com/adaelon/Fluid/releases/latest/download/install.sh | sh
#
# Env overrides:
#   FLUID_VERSION      release tag to install (e.g. v0.1.0); default: latest
#   FLUID_INSTALL_DIR  target dir; default: /usr/local/bin if writable, else ~/.local/bin
set -eu

REPO="adaelon/Fluid"

os=$(uname -s)
arch=$(uname -m)

case "$os" in
  Linux) os_tag="linux" ;;
  Darwin) os_tag="macos" ;;
  *)
    echo "不支持的系统: $os(Windows 请直接下载 fluid-windows-x86_64.exe)" >&2
    exit 1
    ;;
esac

case "$arch" in
  x86_64 | amd64) arch_tag="x86_64" ;;
  arm64 | aarch64) arch_tag="arm64" ;;
  *)
    echo "不支持的架构: $arch" >&2
    exit 1
    ;;
esac

# Guard the one combination the release matrix doesn't build.
if [ "$os_tag" = "linux" ] && [ "$arch_tag" = "arm64" ]; then
  echo "暂无 linux/arm64 预编译包,请从源码构建(见 README)。" >&2
  exit 1
fi

asset="fluid-${os_tag}-${arch_tag}"

if [ -n "${FLUID_VERSION:-}" ]; then
  url="https://github.com/${REPO}/releases/download/${FLUID_VERSION}/${asset}"
else
  url="https://github.com/${REPO}/releases/latest/download/${asset}"
fi

# Pick an install dir: explicit override → /usr/local/bin if writable → ~/.local/bin.
if [ -n "${FLUID_INSTALL_DIR:-}" ]; then
  dir="$FLUID_INSTALL_DIR"
elif [ -w /usr/local/bin ]; then
  dir="/usr/local/bin"
else
  dir="$HOME/.local/bin"
fi
mkdir -p "$dir"

target="$dir/fluid"
echo "下载 $asset → $target"
curl -fSL "$url" -o "$target"
chmod +x "$target"

echo "已安装: $target"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "提示: $dir 不在 PATH 中,请加入:export PATH=\"$dir:\$PATH\"" ;;
esac
echo
echo "运行:  fluid /path/to/project"
echo "(后端+前端同端口启动,默认自动打开 http://127.0.0.1:7878)"
