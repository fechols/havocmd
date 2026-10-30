use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for path in ["build.rs", "link.ld", "include/musicdisk_assets.h"] {
        println!("cargo:rerun-if-changed={path}");
    }
    for name in ["MOS_CLANG", "LLVM_MOS", "HOME"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    // The verified packer emits the C reference header. Derive both runtimes'
    // constants from that one authenticated directory, without a second format.
    let header = fs::read_to_string(root.join("include/musicdisk_assets.h")).unwrap();
    let value = |name: &str| -> String {
        let prefix = format!("#define {name} ");
        header.lines().find_map(|line| line.strip_prefix(&prefix))
            .unwrap_or_else(|| panic!("missing {name}; package full content first"))
            .trim_end_matches("UL").trim_end_matches('u').to_owned()
    };
    assert_eq!(value("MUSICDISK_SAMPLE_RATE"), "16000", "package the complete16k content first");
    assert_eq!(value("MUSICDISK_ASSETS_READY"), "1");
    let playlist = header.lines().find(|line| line.contains("musicdisk_playlist[21]")).unwrap();
    let playlist = playlist.split_once('{').unwrap().1.split_once('}').unwrap().0;
    let generated = format!(
        "pub const MUSICDISK_DIRECTORY_CRC:u32={};\npub const FORMAT_VERSION:u8={};\npub const TRACK_COUNT:u8=21;\npub const ASSET_COUNT:u8=42;\npub const SAMPLE_RATE:u32=16000;\npub const PLAYLIST:[u8;21]=[{}];\n",
        value("MUSICDISK_DIRECTORY_CRC"), value("MUSICDISK_FORMAT_VERSION"), playlist);
    fs::write(out.join("assets.rs"), generated).unwrap();
    let sdk = env::var("LLVM_MOS").unwrap_or_else(|_| format!("{}/.local/llvm-mos", env::var("HOME").unwrap()));
    let compiler = env::var("MOS_CLANG").unwrap_or_else(|_| format!("{sdk}/bin/mos-c64-clang"));
    for name in ["startup", "tilt", "audio_step", "audio_fast", "timer", "memory", "bank", "pano_copy"] {
        let path = root.join(format!("asm/{name}.S"));
        println!("cargo:rerun-if-changed={}", path.display());
        let object = out.join(format!("{name}.o"));
        assert!(Command::new(&compiler).args(["-c", "-g", "-DHAVOC_PRODUCTION=1", "-o"])
            .arg(&object).arg(&path).status().unwrap().success(), "assembly failed: {name}");
        println!("cargo:rustc-link-arg={}", object.display());
    }
    for name in ["tilt_pixels.inc"] {
        println!("cargo:rerun-if-changed=asm/{name}");
    }
    // This player disables IRQs. Preserve its110-byte whole-program ZP pool;
    // sueme64's IRQ-specific -mlto-zp=0 policy does not apply here.
    println!("cargo:rustc-link-arg=-mlto-zp=110");
    println!("cargo:rustc-link-arg=-Oz");
    println!("cargo:rustc-link-arg=-T");
    println!("cargo:rustc-link-arg={}", root.join("link.ld").display());
    println!("cargo:rustc-link-arg=-Wl,--emit-relocs");
    println!("cargo:rustc-link-arg=-Wl,-Map,{}", root.join("build/musicdisk-rust.map").display());
}
