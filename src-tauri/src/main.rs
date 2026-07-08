// Tempo desktop binary entry point.
//
// `windows_subsystem = "windows"` (release only) prevents a console window from
// flashing behind the GUI on Windows. All real logic lives in the library
// (`tempo_lib::run`) so it can be unit-tested and reused on mobile targets.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tempo_lib::run();
}
