// Sem consola em release no Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    renderpro_lib::run()
}
