// Release builds get no console window; debug builds keep one for easy printf debugging.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    stlauncher_lib::run()
}
