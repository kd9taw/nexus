cmake_minimum_required(VERSION 3.20)

# Tempo: standalone FT1 modem library (decoupled from Qt / WSJT-X).
# Compiles ONLY the FT1 4-CPM turbo modem + minimal shared infra into
# static + shared libft1, linking FFTW3 single precision. NO Qt, NO GUI.
project(libft1 LANGUAGES Fortran C CXX)

# ---------------------------------------------------------------------------
# WSJT-X-derived modem source. The complete corresponding source (GPL-3.0 §6)
# is vendored in-tree at libft1/vendor/wsjtx/ and used by DEFAULT, so a fresh
# clone builds the modem with no external checkout. See vendor/wsjtx/README.md
# for provenance and license.
#
# To build against a different WSJT-X tree instead, override at configure time:
#     -DWX=/path/to/wsjtx-source        (its lib/ must hold the sources)
# ---------------------------------------------------------------------------
set(WX "${CMAKE_CURRENT_SOURCE_DIR}/vendor/wsjtx" CACHE PATH "WSJT-X-derived modem source tree (contains lib/)")
if(NOT IS_DIRECTORY "${WX}/lib")
    message(FATAL_ERROR
        "WSJT-X modem source not found at ${WX}/lib. The vendored copy lives at "
        "libft1/vendor/wsjtx/; override with -DWX=/path/to/wsjtx-source if needed.")
endif()
set(LIB "${WX}/lib")

# ---------------------------------------------------------------------------
# FFTW3 single precision (used by four2a.f90 via the fftw3 module / fftw3.f03).
#
# Native (host == target): locate fftw3f via pkg-config, Boost via find_package.
# Cross-compiling to Windows from a non-Windows host (CMAKE_CROSSCOMPILING set by
# the MinGW toolchain file): pkg-config / find_package would pick up HOST libs,
# so instead point at the statically cross-built FFTW3f at FFTW_MINGW_PREFIX and
# use the host's header-only Boost (crc.hpp is header-only, arch-independent).
# ---------------------------------------------------------------------------
if(CMAKE_CROSSCOMPILING AND CMAKE_SYSTEM_NAME STREQUAL "Windows")
    # Cross build: use the MinGW static FFTW3f from the toolchain file.
    if(NOT DEFINED FFTW_MINGW_PREFIX)
        set(FFTW_MINGW_PREFIX "/tmp/fftw-mingw")
    endif()
    if(NOT EXISTS "${FFTW_MINGW_PREFIX}/lib/libfftw3f.a")
        message(FATAL_ERROR
            "Cross build needs MinGW FFTW3f at ${FFTW_MINGW_PREFIX}/lib/libfftw3f.a "
            "(build it with ./configure --host=x86_64-w64-mingw32 --enable-float "
            "--enable-static --disable-shared --prefix=${FFTW_MINGW_PREFIX}).")
    endif()
    set(FFTW3F_INCLUDE_DIRS "${FFTW_MINGW_PREFIX}/include")
    set(FFTW3F_LIBRARY_DIRS "${FFTW_MINGW_PREFIX}/lib")
    set(FFTW3F_LIBRARIES    "${FFTW_MINGW_PREFIX}/lib/libfftw3f.a")
    # Header-only Boost CRC: use the host include tree (no compiled Boost needed).
    add_library(boost_crc_headeronly INTERFACE)
    set(BOOST_CRC_TARGET boost_crc_headeronly)
else()
    find_package(PkgConfig REQUIRED)
    pkg_check_modules(FFTW3F REQUIRED fftw3f)
    # Boost provides <boost/crc.hpp> for crc14.cpp (header-only).
    find_package(Boost REQUIRED)
    set(BOOST_CRC_TARGET Boost::boost)
endif()

# ---------------------------------------------------------------------------
# Fortran build flags.
#  -fallow-argument-mismatch : legacy WSJT-X code passes mismatched ranks/types
#                              (e.g. four2a equivalence). gfortran 10+ requires
#                              this to downgrade to a warning.
# ---------------------------------------------------------------------------
set(CMAKE_Fortran_FLAGS "${CMAKE_Fortran_FLAGS} -fallow-argument-mismatch -ffree-line-length-none")

# Generated .mod files go in a per-target private dir (set below). CMake's
# built-in Fortran module dependency scanner resolves module `use` ordering
# automatically. Per-target dirs avoid a parallel-build race between the
# static and shared library targets writing the same .mod files.

# ---------------------------------------------------------------------------
# Minimal source list. Add a source only when the compiler/linker demands it.
# ---------------------------------------------------------------------------
set(LIBFT1_SOURCES
    # --- C ABI wrapper (this project) ---
    ${CMAKE_CURRENT_SOURCE_DIR}/ft1_cabi.f90

    # --- DX1-S non-coherent M-FSK mode (this project) ---
    #     params first (defines module dx1_params used by the rest).
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/dx1_params.f90    # module dx1_params
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/gen_dx1wave.f90   # dx1_encode_msg, gen_dx1wave
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/dx1_detect.f90    # dx1_detect (1920-pt FFT)
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/dx1_llr.f90       # dx1_llr (log-sum-exp)
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/dx1_sync.f90      # dx1_sync (chirp correlate)
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/dx1_decode.f90    # dx1_decode (single carrier)
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/dx1_decode_all.f90 # dx1_decode_all (full-band scan)

    # --- FT1 modem (lib/ft1) ---
    ${LIB}/ft1/cpm_trellis.f90            # module cpm_trellis_mod
    ${LIB}/ft1/matched_filter_bank.f90    # module matched_filter_bank_mod
    ${LIB}/ft1/bcjr_cpm.f90
    ${LIB}/ft1/ft1_interleave.f90
    ${LIB}/ft1/genft1.f90
    ${LIB}/ft1/gen_ft1wave.f90
    ${LIB}/ft1/ft1_downsample.f90
    ${LIB}/ft1/turbo_decode_ft1.f90       # also defines bpdecode174_91_beliefs,
                                          #   ft1_interleave_real,
                                          #   compute_branch_metrics_all,
                                          #   force_sync_metrics, ft1_joint_turbo_harq
    ${LIB}/ft1/ft1_demod_bcjr.f90         # ft1_demod_bcjr + ft1_bm_prep (per-frame
                                          #   branch-metric prep for joint turbo HARQ)
    ${LIB}/ft1/ft1_rv_detect.f90          # ft1_rv_detect (coherent RV discriminator)

    # --- FT1 full RX acquisition pipeline ---
    ${LIB}/ft1_decode.f90                 # module ft1_decode: OO decoder; also
                                          #   defines sync1d_ft1, subtractft1,
                                          #   ft1_freq_est_acorr
    ${LIB}/ft1/ft1_sync.f90               # ft1_sync (Costas candidate search),
                                          #   ft1_baseline
    ${LIB}/ft1/ir_harq_combine.f90        # module ir_harq_combine_mod (IR-HARQ)
    ${LIB}/ft1/ldpc348_91.f90             # module ldpc348_91_mod (bpdecode_ext);
                                          #   includes ../ft8/ldpc_174_91_c_parity.f90
    ${LIB}/timer_module.f90               # module timer_module (timer => null_timer
                                          #   no-op). NOT timer_impl.f90 -- that
                                          #   pulls in iso_c_utilities + a C callback.

    # --- shared CPM pulse (lib/ft2) ---
    ${LIB}/ft2/gfsk_pulse.f90

    # --- FEC / FT8 shared (lib/ft8) ---
    ${LIB}/ft8/encode174_91.f90           # includes ldpc_174_91_c_generator.f90
    ${LIB}/ft8/encode174_91_nocrc.f90     # includes ldpc_174_91_c_generator.f90
    ${LIB}/ft8/osd174_91.f90
    ${LIB}/ft8/bpdecode174_91.f90         # standalone BP decoder (ldpc348_91
                                          #   bpdecode_ext fallback); includes
                                          #   ldpc_174_91_c_parity.f90
    ${LIB}/ft8/chkcrc14a.f90
    ${LIB}/ft8/get_crc14.f90

    # --- FT8 native decode (lib/ft8) : Nexus ---
    #     Full-frame decode via the core primitives (ft8apset -> sync8 -> ft8b),
    #     wrapped behind a C ABI in ft8_cabi.f90 (no nzhsym/shmem machinery; the
    #     a7 cross-cycle AP table IS wired in via ft8_a7.f90 below).
    ${CMAKE_CURRENT_SOURCE_DIR}/ft8_cabi.f90
    ${LIB}/ft8/ft8_a7.f90                 # module ft8_a7: cross-cycle AP table
                                          #   (ft8_a7_save + ft8_a7d replay)
    ${LIB}/ft8/genft8.f90                 # genft8 + entry get_ft8_tones_from_77bits
    ${LIB}/ft8/gen_ft8wave.f90            # tones -> waveform (also used by subtractft8)
    ${LIB}/ft8/ft8apset.f90               # a-priori symbol setup (calls stdcall)
    ${CMAKE_CURRENT_SOURCE_DIR}/ft8_stdcall.f90  # vendored stdcall (avoids Q65 stack)
    ${LIB}/ft8/sync8.f90                  # Costas candidate search (calls get_spectrum_baseline)
    ${LIB}/ft8/sync8d.f90                 # fine sync / symbol metrics (called by ft8b)
    ${LIB}/ft8/get_spectrum_baseline.f90  # calls baseline, nuttal_window, four2a
    ${LIB}/ft8/baseline.f90               # spectral baseline fit
    ${LIB}/ft8/ft8_downsample.f90         # dd -> complex baseband (called by ft8b)
    ${LIB}/ft8/ft8b.f90                   # per-candidate decode (+ internal subtract);
                                          #   normalizebmet is contained within
    ${LIB}/ft8/subtractft8.f90            # signal subtraction (calls gen_ft8wave, peakup)
    ${LIB}/ft8/decode174_91.f90           # bp+osd decode (called by ft8b)
    ${LIB}/ft8/twkfreq1.f90               # fine frequency tweak (called by ft8b + ft4)
    ${LIB}/peakup.f90                     # parabolic peak interpolation (subtractft8)

    # --- FT4 native decode (lib/ft4 + ft4_decode.f90) : Nexus ---
    #     Clean OO decoder (getcandidates4 -> sync4d -> get_ft4_bitmetrics ->
    #     decode174_91 -> subtractft4), wrapped behind a C ABI in ft4_cabi.f90.
    ${CMAKE_CURRENT_SOURCE_DIR}/ft4_cabi.f90
    ${LIB}/ft4_decode.f90                 # module ft4_decode (OO decoder)
    ${LIB}/ft4/genft4.f90                 # genft4 + entry get_ft4_tones_from_77bits
    ${LIB}/ft4/gen_ft4wave.f90            # tones -> waveform (also used by subtractft4)
    ${LIB}/ft4/getcandidates4.f90         # candidate search (calls ft4_baseline)
    ${LIB}/ft4/ft4_baseline.f90           # spectral baseline
    ${LIB}/ft4/ft4_downsample.f90         # dd -> complex baseband
    ${LIB}/ft4/sync4d.f90                 # sync power metric
    ${LIB}/ft4/get_ft4_bitmetrics.f90     # bit metrics (calls normalizebmet from ft8b)
    ${LIB}/ft4/subtractft4.f90            # signal subtraction (calls gen_ft4wave)

    # --- 77-bit message packing (lib/77bit) ---
    # NOTE: call_to_c28, encode77, free_text, hash22calc, nonstd_to_c58, test28
    # are standalone TEST PROGRAMS (each has its own `program main`), NOT library
    # sources -- including them caused "multiple definition of main". The real
    # pack/unpack subroutines (pack77, unpack77, mp_short_*, hash22, ...) all
    # live inside packjt77.f90's module. parse77/my_hash are not in the genft1
    # encode->decode path, so they are omitted.
    ${LIB}/77bit/packjt77.f90             # module packjt77

    # --- misc shared (lib) ---
    ${LIB}/chkcall.f90                    # called by pack77 (callsign validation)
    ${LIB}/fmtmsg.f90                     # message formatting (pack/unpack)
    ${LIB}/grid2deg.f90                   # grid <-> lat/long (unpack77 grid)
    ${LIB}/deg2grid.f90                   # grid <-> lat/long (unpack77 grid)
    ${LIB}/packjt.f90                     # module packjt (used by packjt77)
    ${LIB}/crc.f90                        # module crc (binds C crc14/get_crc14)
    ${LIB}/db.f90                         # power -> dB (ft8_a7d SNR estimate)
    ${LIB}/indexx.f90
    ${LIB}/platanh.f90
    ${LIB}/nuttal_window.f90              # spectral window (ft1_sync)
    ${LIB}/pctile.f90                     # percentile (ft1_baseline); calls shell
    ${LIB}/polyfit.f90                    # polynomial fit (ft1_sync baseline)
    ${LIB}/determ.f90                     # matrix determinant (polyfit)
    ${LIB}/shell.f90                      # shellsort (pctile)
    ${LIB}/fftw3mod.f90                   # module fftw3 (wraps fftw3.f03)
    ${LIB}/four2a.f90                     # FFT via FFTW3f

    # --- C / C++ infra ---
    ${LIB}/crc14.cpp                      # Boost CRC-14 (crc14 / crc14_check)
)

# ---------------------------------------------------------------------------
# Include directories for Fortran `include` statements:
#   ft1/ft1_params.f90          -> -I lib/ft1
#   ldpc_174_91_c_generator.f90 -> -I lib/ft8
#   ldpc_174_91_c_parity.f90    -> -I lib/ft8
#   fftw3.f03                   -> -I <fftw include dir> (usually /usr/include)
# ---------------------------------------------------------------------------
set(LIBFT1_INCLUDE_DIRS
    ${LIB}/ft1
    ${LIB}/ft8
    ${LIB}
    ${FFTW3F_INCLUDE_DIRS}   # fftw3.f03 (native: from pkg-config /usr/include;
                             #            cross: from the MinGW FFTW prefix)
)
if(NOT CMAKE_CROSSCOMPILING)
    # fftw3.f03 lives in /usr/include on this native system; the MinGW prefix
    # ships its own copy (already in FFTW3F_INCLUDE_DIRS) for the cross build.
    list(APPEND LIBFT1_INCLUDE_DIRS /usr/include)
endif()
# For the cross build, crc14.cpp does #include <boost/crc.hpp> (header-only).
# We expose only the host Boost headers, and via -idirafter so they are searched
# AFTER the MinGW system headers — putting /usr/include on the normal -I path
# would shadow MinGW's libc headers (wchar.h, stdlib.h, ...) and break the build.
# fftw3.f03 for Fortran comes from the MinGW FFTW prefix (in FFTW3F_INCLUDE_DIRS).
if(CMAKE_CROSSCOMPILING AND CMAKE_SYSTEM_NAME STREQUAL "Windows")
    set(BOOST_HEADER_FLAG "-idirafter" "/usr/include")
endif()

# ---------------------------------------------------------------------------
# Static + shared libft1.
# ---------------------------------------------------------------------------
add_library(ft1_static STATIC ${LIBFT1_SOURCES})
add_library(ft1_shared SHARED ${LIBFT1_SOURCES})
set_target_properties(ft1_static PROPERTIES OUTPUT_NAME ft1)
set_target_properties(ft1_shared PROPERTIES OUTPUT_NAME ft1)

foreach(tgt ft1_static ft1_shared)
    set(MODDIR "${CMAKE_CURRENT_BINARY_DIR}/modules/${tgt}")
    file(MAKE_DIRECTORY "${MODDIR}")
    set_target_properties(${tgt} PROPERTIES Fortran_MODULE_DIRECTORY "${MODDIR}")
    target_include_directories(${tgt} PRIVATE ${LIBFT1_INCLUDE_DIRS} "${MODDIR}")
    target_include_directories(${tgt} PUBLIC  ${CMAKE_CURRENT_SOURCE_DIR}/include)
    target_link_libraries(${tgt} PUBLIC ${FFTW3F_LIBRARIES})
    target_link_directories(${tgt} PUBLIC ${FFTW3F_LIBRARY_DIRS})
    target_link_libraries(${tgt} PUBLIC ${BOOST_CRC_TARGET})
    # crc14.cpp #include <boost/crc.hpp>: add host Boost headers as a last-resort
    # search path (cross build only; empty natively).
    if(BOOST_HEADER_FLAG)
        target_compile_options(${tgt} PRIVATE
            $<$<COMPILE_LANGUAGE:CXX>:${BOOST_HEADER_FLAG}>)
    endif()
endforeach()

# ---------------------------------------------------------------------------
# Verification executable A: build the EXISTING ft1_test.f90 against libft1.
# It also needs the C Gaussian-noise generators gran.c / sgran.c (used only by
# the test harness, not the modem itself).
# ---------------------------------------------------------------------------
add_executable(ft1_test_standalone
    ${LIB}/ft1/ft1_test.f90
    ${LIB}/gran.c
    ${LIB}/sgran.c
    ${LIB}/init_random_seed.c            # sgran() calls init_random_seed()
)
target_include_directories(ft1_test_standalone PRIVATE
    ${LIBFT1_INCLUDE_DIRS}
    "${CMAKE_CURRENT_BINARY_DIR}/modules/ft1_static")
target_link_libraries(ft1_test_standalone PRIVATE ft1_static)

# ---------------------------------------------------------------------------
# Verification executable A2: DX1-S AWGN + Rayleigh-fading decode-rate sweep.
# Like ft1_test_standalone: links ft1_static + the C Gaussian-noise generators.
# ---------------------------------------------------------------------------
add_executable(dx1_test_standalone
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/dx1_test.f90
    ${LIB}/gran.c
    ${LIB}/sgran.c
    ${LIB}/init_random_seed.c
)
target_include_directories(dx1_test_standalone PRIVATE
    ${LIBFT1_INCLUDE_DIRS}
    "${CMAKE_CURRENT_BINARY_DIR}/modules/ft1_static")
target_link_libraries(dx1_test_standalone PRIVATE ft1_static)

# ---------------------------------------------------------------------------
# Verification executable A3: DX1-S full-passband acquisition (dx1_decode_all):
# 3 signals at distinct carriers + times -> all decoded; plus a scan-budget gate.
# ---------------------------------------------------------------------------
add_executable(dx1_band_test_standalone
    ${CMAKE_CURRENT_SOURCE_DIR}/dx1/dx1_band_test.f90
    ${LIB}/gran.c
    ${LIB}/sgran.c
    ${LIB}/init_random_seed.c
)
target_include_directories(dx1_band_test_standalone PRIVATE
    ${LIBFT1_INCLUDE_DIRS}
    "${CMAKE_CURRENT_BINARY_DIR}/modules/ft1_static")
target_link_libraries(dx1_band_test_standalone PRIVATE ft1_static)

# ---------------------------------------------------------------------------
# Verification executable B: tiny C harness exercising the C ABI.
# ---------------------------------------------------------------------------
add_executable(roundtrip ${CMAKE_CURRENT_SOURCE_DIR}/../tests/roundtrip.c)
target_link_libraries(roundtrip PRIVATE ft1_static m)
# gfortran runtime needed when linking Fortran static lib into a C executable.
target_link_libraries(roundtrip PRIVATE gfortran)

# ---------------------------------------------------------------------------
# Verification executable C: full RX acquisition decode via the C ABI.
# ---------------------------------------------------------------------------
add_executable(acquire ${CMAKE_CURRENT_SOURCE_DIR}/../tests/acquire.c)
target_link_libraries(acquire PRIVATE ft1_static m gfortran)

# ---------------------------------------------------------------------------
# Verification executable D: native FT8 full-frame acquisition + decode parity.
# ---------------------------------------------------------------------------
add_executable(ft8_acquire ${CMAKE_CURRENT_SOURCE_DIR}/../tests/ft8_acquire.c)
target_link_libraries(ft8_acquire PRIVATE ft1_static m gfortran)

# ---------------------------------------------------------------------------
# Verification executable E: native FT4 full-frame acquisition + decode parity.
# ---------------------------------------------------------------------------
add_executable(ft4_acquire ${CMAKE_CURRENT_SOURCE_DIR}/../tests/ft4_acquire.c)
target_link_libraries(ft4_acquire PRIVATE ft1_static m gfortran)
