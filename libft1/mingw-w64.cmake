# CMake toolchain file: cross-compile libft1 (Fortran + C + C++ + FFTW3f) from a
# Linux host to Windows x86_64 using the MinGW-w64 GNU toolchain.
#
# Usage:
#   cmake -S libft1 -B libft1/build-win \
#         -DCMAKE_TOOLCHAIN_FILE=$PWD/libft1/mingw-w64.cmake \
#         -DCMAKE_BUILD_TYPE=Release -G Ninja
#
# FFTW3f is expected as a static MinGW build at FFTW_MINGW_PREFIX
# (default /tmp/fftw-mingw), produced with:
#   ./configure --host=x86_64-w64-mingw32 --enable-float --enable-static \
#               --disable-shared --prefix=/tmp/fftw-mingw && make && make install
#
# Threading: the default (unsuffixed) x86_64-w64-mingw32-* compilers on this box
# are the win32 thread model, matching the win32 libgfortran.a and Rust's
# x86_64-pc-windows-gnu target. We deliberately use the unsuffixed names so the
# whole stack (libft1, gfortran runtime, Rust linker) agrees on one thread model.

set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(TOOLCHAIN_PREFIX x86_64-w64-mingw32)

set(CMAKE_C_COMPILER       ${TOOLCHAIN_PREFIX}-gcc)
set(CMAKE_CXX_COMPILER     ${TOOLCHAIN_PREFIX}-g++)
set(CMAKE_Fortran_COMPILER ${TOOLCHAIN_PREFIX}-gfortran)
set(CMAKE_RC_COMPILER      ${TOOLCHAIN_PREFIX}-windres)

# Where to look for the target environment / libraries.
set(CMAKE_FIND_ROOT_PATH /usr/${TOOLCHAIN_PREFIX})

# Search for programs on the host, but headers/libs only in the target root.
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# Cross-built FFTW3f (single precision). Override with -DFFTW_MINGW_PREFIX=...
if(NOT DEFINED FFTW_MINGW_PREFIX)
    set(FFTW_MINGW_PREFIX "/tmp/fftw-mingw")
endif()
set(FFTW_MINGW_PREFIX "${FFTW_MINGW_PREFIX}" CACHE PATH "MinGW FFTW3f install prefix")

# Statically link the GCC/gfortran/stdc++/quadmath/winpthread runtime INTO each
# executable so the cross-built test exes are self-contained on Windows (no
# libgfortran-5.dll / libstdc++-6.dll / libwinpthread-1.dll to ship alongside).
# Matches win_smoke.exe's "runs anywhere" property. Executables only — the shared
# libft1.dll is a DLL by design and is unaffected. FFTW3f is already static, so
# -static yields a fully standalone exe.  (_INIT seeds the flag at first configure,
# so a fresh build dir is needed for it to take.)
set(CMAKE_EXE_LINKER_FLAGS_INIT "-static")
