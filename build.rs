fn main() {
    // Igual que la app de escritorio: las DLLs que importa el exe (las resuelve Windows antes
    // de main) se buscan solo en System32 (0x800 = LOAD_LIBRARY_SEARCH_SYSTEM32).
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
}
