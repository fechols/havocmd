use havocmd::{panorama, volume};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("havocmd: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bake-art") if args.len() <= 3 => bake_art(
            Path::new(args.get(1).map_or("assets/splashes", String::as_str)),
            Path::new(args.get(2).map_or("target/art", String::as_str)),
        ),
        Some("pack") if args.len() == 3 => pack_manifest(Path::new(&args[1]), Path::new(&args[2])),
        Some("check") if args.len() == 2 => check(Path::new(&args[1])),
        Some("budget") if args.len() <= 2 => budget(args.get(1).map(Path::new)),
        None | Some("help" | "--help" | "-h") => {
            println!(
                "HAVOC Musicdisk content tools\n\n  bake-art [assets/splashes] [target/art]\n  pack <asset-manifest.json> <output-directory>\n  check <volume-directory>\n  budget [music-catalog.json]\n\nPack manifest: {{\"assets\":[{{\"id\":3,\"kind\":\"panorama\",\"name\":\"lowland-approach\",\"path\":\"lowland-approach.bin\",\"metadata\":[960,1024,16,1]}}]}}\nPaths are relative to the manifest; metadata is four optional u32 words.\nArt v2 .bin files contain a 32 KiB seed followed by native 16x8 tiles, 4 bits per pixel.\n.cache.bin files duplicate the tile-aligned 256x256 seed for diagnostics."
            );
            Ok(())
        }
        _ => Err("invalid command or argument count; run havocmd --help".into()),
    }
}

fn bake_art(source_dir: &Path, output: &Path) -> Result<()> {
    let mut sources = Vec::new();
    for entry in fs::read_dir(source_dir)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|v| v.to_str())
                .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"))
        {
            sources.push(path);
        }
    }
    sources.sort();
    if sources.is_empty() || sources.len() > 21 {
        return Err("expected between 1 and 21 panorama source images".into());
    }
    let mut names = BTreeSet::new();
    let mut slugs = Vec::new();
    let mut dimensions = Vec::new();
    for source in &sources {
        let name = panorama::slug(
            source
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("source filename is not UTF-8")?,
        );
        if name.is_empty() || !names.insert(name.clone()) {
            return Err(format!("empty or duplicate panorama slug: {name}").into());
        }
        slugs.push(name);
        dimensions.push(image::image_dimensions(source)?);
    }
    let (scale, layouts) = panorama::plan_layouts(
        &dimensions,
        panorama::PIXEL_ASPECT,
        panorama::PER_PICTURE_BYTES,
        panorama::TOTAL_ART_BYTES,
    )?;
    fs::create_dir_all(output)?;
    let mut assets = Vec::new();
    for (index, ((source, slug), layout)) in
        sources.iter().zip(&slugs).zip(layouts.iter()).enumerate()
    {
        let image = image::open(source)?.to_rgb8();
        let indexed = panorama::quantize(&image, *layout);
        let bytes = indexed.pack_tiles();
        let file_name = format!("{slug}.bin");
        fs::write(output.join(&file_name), &bytes)?;
        fs::write(
            output.join(format!("{slug}.cache.bin")),
            indexed.cache_256(),
        )?;
        indexed
            .preview(panorama::PIXEL_ASPECT)
            .save(output.join(format!("{slug}.preview.png")))?;
        let id = panorama::TRACK_SLUGS
            .iter()
            .position(|candidate| candidate == slug)
            .map_or(1000 + index, |id| id) as u16;
        let metadata = [
            layout.width,
            layout.height,
            layout.storage_width,
            layout.storage_height,
        ];
        let details = json!({
            "format":"havoc-panorama-v2", "id":id, "name":slug,
            "source":source.file_name().unwrap().to_string_lossy(), "source_size":dimensions[index],
            "source_crc32":format!("{:08x}",volume::crc32(&fs::read(source)?)),
            "logical_size":[layout.width,layout.height], "storage_size":[layout.storage_width,layout.storage_height],
            "pixel_aspect":panorama::PIXEL_ASPECT, "common_scale":scale,
            "palette":"ultimate-default", "palette_rgb":panorama::PALETTE,
            "palette_source":"https://github.com/GideonZ/1541ultimate/blob/master/software/u64/u64_config.cc",
            "quantizer":"Lanczos3 resample; linear-light serpentine Floyd-Steinberg",
            "seed_bytes":panorama::SEED_BYTES, "seed_origin":indexed.seed_origin(),
            "tile_offset":panorama::SEED_BYTES,
            "tile_size":[16,8], "tile_bytes":64, "tile_order":"column-major",
            "pixel_order":"packed X columns, eight rows; even X low nibble, odd X high nibble",
            "payload":file_name, "payload_bytes":bytes.len(), "payload_crc32":format!("{:08x}",volume::crc32(&bytes)),
            "prototype_cache":{"file":format!("{slug}.cache.bin"),"size":[256,256],"bytes":32768,"crop":"tile-aligned center, edge clamped", "origin":indexed.seed_origin(), "layout":"128 columns x 256 bytes; adjacent X pixels, even X low nibble"}
        });
        fs::write(
            output.join(format!("{slug}.json")),
            serde_json::to_vec_pretty(&details)?,
        )?;
        assets.push(
            json!({"id":id,"kind":"panorama","name":slug,"path":file_name,"metadata":metadata}),
        );
        println!(
            "{slug}: {}x{} logical, {} bytes",
            layout.width,
            layout.height,
            bytes.len()
        );
    }
    let total: usize = layouts.iter().map(|l| l.bytes).sum();
    fs::write(
        output.join("assets.json"),
        serde_json::to_vec_pretty(&json!({"format":"havoc-asset-manifest-v1","assets":assets}))?,
    )?;
    fs::write(
        output.join("budget.json"),
        serde_json::to_vec_pretty(
            &json!({"picture_count":sources.len(),"common_scale":scale,"pixel_aspect":panorama::PIXEL_ASPECT,"art_bytes":total,"art_budget":panorama::TOTAL_ART_BYTES,"per_picture_cap":panorama::PER_PICTURE_BYTES}),
        )?,
    )?;
    println!(
        "{} panoramas: {total}/{} art bytes; common scale {scale:.6}",
        sources.len(),
        panorama::TOTAL_ART_BYTES
    );
    Ok(())
}

fn field_u32(value: &Value, field: &str) -> Result<u32> {
    value[field]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("invalid {field}: expected u32").into())
}

fn pack_manifest(manifest: &Path, output: &Path) -> Result<()> {
    let document: Value = serde_json::from_slice(&fs::read(manifest)?)?;
    let array = document
        .get("assets")
        .unwrap_or(&document)
        .as_array()
        .ok_or("manifest requires an assets array")?;
    if array.is_empty() || array.len() > volume::MAX_ASSETS {
        return Err("manifest requires 1..=42 assets".into());
    }
    let base = manifest.parent().unwrap_or(Path::new("."));
    let mut inputs = Vec::new();
    for value in array {
        let id = u16::try_from(field_u32(value, "id")?).map_err(|_| "asset id exceeds u16")?;
        let kind = match value["kind"].as_str() {
            Some("track") => volume::AssetKind::Track,
            Some("panorama") => volume::AssetKind::Panorama,
            _ => return Err("asset kind must be track or panorama".into()),
        };
        let name = value["name"]
            .as_str()
            .ok_or("asset name is required")?
            .to_owned();
        let path = base.join(value["path"].as_str().ok_or("asset path is required")?);
        let mut metadata = [0u32; 4];
        if let Some(values) = value.get("metadata") {
            let values = values
                .as_array()
                .filter(|v| v.len() == 4)
                .ok_or("metadata must contain four u32 words")?;
            for (i, v) in values.iter().enumerate() {
                metadata[i] = v
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or("metadata value exceeds u32")?;
            }
        }
        inputs.push(volume::AssetInput {
            id,
            kind,
            name,
            data: fs::read(path)?,
            metadata,
        });
    }
    let packed = volume::pack(&inputs)?;
    volume::validate_set(&packed.volumes.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
    fs::create_dir_all(output)?;
    for (i, bytes) in packed.volumes.iter().enumerate() {
        fs::write(volume_path(output, i), bytes)?;
    }
    // A smaller release must not leave a previous build's higher volumes behind.
    for i in packed.volumes.len()..volume::MAX_VOLUME_COUNT {
        let stale = volume_path(output, i);
        if stale.exists() {
            fs::remove_file(&stale)?;
            println!("Removed stale {}", stale.display());
        }
    }
    println!(
        "Packed {} assets into {} verified 16 MiB volumes at {}",
        inputs.len(),
        packed.volumes.len(),
        output.display()
    );
    Ok(())
}

fn volume_path(directory: &Path, index: usize) -> PathBuf {
    directory.join(format!("havocmd-{}.reu", index + 1))
}
fn check(directory: &Path) -> Result<()> {
    let first = fs::read(volume_path(directory, 0))?;
    let count = volume::parse_header(&first)?.volume_count as usize;
    let mut volumes = vec![first];
    for i in 1..count {
        volumes.push(fs::read(volume_path(directory, i))?);
    }
    for i in count..volume::MAX_VOLUME_COUNT {
        if volume_path(directory, i).exists() {
            return Err(format!("stale volume {} beyond the recorded count {count}", i + 1).into());
        }
    }
    let entries = volume::validate_set(&volumes.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
    println!(
        "Verified {count} 16 MiB volumes, {} assets, matching directories and all checksums",
        entries.len()
    );
    Ok(())
}

fn budget(catalog: Option<&Path>) -> Result<()> {
    println!(
        "Volume payload capacity: {} bytes (up to four 16 MiB files minus their headers)",
        volume::TOTAL_PAYLOAD_BYTES
    );
    println!(
        "Art budget: {} bytes; per picture: {} bytes; pixel aspect: {}",
        panorama::TOTAL_ART_BYTES,
        panorama::PER_PICTURE_BYTES,
        panorama::PIXEL_ASPECT
    );
    if let Some(catalog) = catalog {
        let document: Value = serde_json::from_slice(&fs::read(catalog)?)?;
        let tracks = document["tracks"]
            .as_array()
            .ok_or("catalog requires tracks array")?;
        let mut bytes = 0u64;
        let mut allocated = 0u64;
        for track in tracks {
            let length = track["expected_asset_bytes"]
                .as_u64()
                .ok_or("track expected_asset_bytes required")?;
            bytes = bytes.checked_add(length).ok_or("catalog size overflow")?;
            allocated = allocated
                .checked_add(length.div_ceil(volume::ALIGNMENT as u64) * volume::ALIGNMENT as u64)
                .ok_or("aligned catalog size overflow")?;
        }
        println!(
            "Catalog estimate: {} {} tracks, {bytes} bytes; {allocated} with volume alignment",
            tracks.len(), document["profile"].as_str().unwrap_or("unknown")
        );
        let remaining = (volume::TOTAL_PAYLOAD_BYTES as u64)
            .checked_sub(allocated)
            .ok_or("estimated soundtrack exceeds four-volume capacity")?;
        println!("Remaining art allocation: {remaining} bytes (verify against final encodings)");
    }
    Ok(())
}
