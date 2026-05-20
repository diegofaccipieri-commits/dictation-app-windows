@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
set "PATH=C:\Program Files\nodejs;C:\Users\giuli\.cargo\bin;C:\Program Files\LLVM\bin;%PATH%"
set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
echo LIBCLANG_PATH is: %LIBCLANG_PATH%
cd /d C:\Users\giuli\dictation-app-windows
npx tauri build
