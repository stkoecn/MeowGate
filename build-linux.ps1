# MeowGate 一键交叉编译 Linux (Debian/Ubuntu/Alpine) 纯静态二进制脚本
Write-Host ">>> 正在为 Linux x86_64 (musl 纯静态) 编译 Release 产物..." -ForegroundColor Cyan

$env:PATH = "C:\Users\winds\scoop\apps\python310\current\Lib\site-packages\ziglang;" + $env:PATH
cargo zigbuild --target x86_64-unknown-linux-musl --release

if ($LASTEXITCODE -eq 0) {
    $binPath = "target\x86_64-unknown-linux-musl\release\meowgate"
    $size = (Get-Item $binPath).Length / 1MB
    Write-Host ">>> 编译成功！产物位置: $binPath (体积: $($size.ToString('F2')) MB)" -ForegroundColor Green
    Write-Host ">>> 此文件为纯静态 ELF 二进制，可直接上传到 Debian 10/11/12 等任何 Linux 系统上运行。" -ForegroundColor Yellow
} else {
    Write-Host ">>> 编译失败，请检查报错输出。" -ForegroundColor Red
}
