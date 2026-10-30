"""Validate complete stereo16k3 recordings and build the musicdisk volumes.

The packer writes as many 16 MiB volumes as the content needs (at most four) and
records the count in every header.

Uses the existing Rust pack/check CLI, then independently verifies its headers,
directory, split assets and checksums before generating the target's expected
directory CRC. Hardware acceptance is deliberately not inferred from this tool.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import struct
import subprocess
import zlib

ROOT = Path(__file__).resolve().parents[1]
VOLUME_BYTES = 16 * 1024 * 1024
HEADER_BYTES = 4096
PAYLOAD_BYTES = VOLUME_BYTES - HEADER_BYTES
ALIGNMENT = 256
FORMAT_VERSION = 2
TRACK_COUNT = 21
SEED_BYTES = 32768
# One player slot holds a song and its panorama (target-c64/src/musicdisk.rs).
SLOT_BYTES = 5 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def document(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def crc32(data):
    return zlib.crc32(data) & 0xffffffff


def align(length):
    return (length + ALIGNMENT - 1) & -ALIGNMENT


def crc8_table():
    values = []
    for byte in range(256):
        value = byte
        for _ in range(8):
            value = ((value << 1) ^ (7 if value & 128 else 0)) & 255
        values.append(value)
    return values


CRC8 = crc8_table()


def validate_track(track, music):
    name = track['name']
    path = music / track['asset']
    data = path.read_bytes()
    meta = document(music / track['metadata'])
    provenance = document(music / (name + '.provenance.json'))
    require(meta['profile'] == 'stereo16k3' and meta['sample_rate'] == 16000 and
            meta['channels'] == 2 and meta['bits_per_sample'] == 3 and
            meta['output_bits_per_sample'] == 8, f'{name}: wrong audio profile')
    require(len(data) == meta['asset_bytes'] == track['expected_asset_bytes'] and
            sha256(data) == meta['asset_sha256'], f'{name}: asset hash/extent mismatch')
    require(sha256((music / (name + '-source.flac')).read_bytes()) == meta['flac_sha256'],
            f'{name}: encoded source differs from the authenticated extraction')
    require(provenance['source_verified_sha256'] == track['source']['sha256'] and
            provenance['conversion']['diagnostic'] is False and
            provenance['conversion']['profile'] == 'stereo16k3' and
            provenance['conversion']['decoded_source_frames'] == track['expected_source_frames'] and
            provenance['conversion']['asset_sha256'] == meta['asset_sha256'],
            f'{name}: full-song source provenance mismatch')
    require(meta['sample_count'] == track['expected_frames'] and
            meta['codebook_count'] == track['codebook_count'], f'{name}: truncated song or altered codebook count')
    require(meta['encoder_training'] == track['encoder_options']['training'] and
            meta['encoder_beam_width'] == track['encoder_options']['beam_width'],
            f'{name}: encoder quality settings differ from the catalog')
    magic, rate, frames, stream_bytes, entries, books, bits, channels = struct.unpack_from('<4sIIIHHBB', data)
    require((magic, rate, frames, entries, books, bits, channels) ==
            (b'DLT5', 16000, track['expected_frames'], 8, track['codebook_count'], 3, 2),
            f'{name}: DLT5 header mismatch')
    # The player decodes whole 3-byte packing groups of four stereo frames and
    # derives each codebook segment with an 8-bit quotient (audio.rs audio_track_read).
    require(frames % 4 == 0, f'{name}: frame count is not a whole number of packing groups')
    require(books <= frames // 4 < 256 * books,
            f'{name}: codebook count {books} leaves segments over 255 groups; the player needs at least {frames // 4 // 256 + 1}')
    require(not any(data[22:32]), f'{name}: nonzero reserved header bytes')
    expected_frames = (track['expected_source_frames'] * 16000 + track['source']['sample_rate'] - 1) // track['source']['sample_rate']
    expected_frames = (expected_frames + 3) // 4 * 4  # the encoder pads the final packing group
    require(frames == expected_frames, f'{name}: source duration not preserved')
    stream_offset = 32 + books * 32
    require(stream_bytes == (frames * 6 + 7) // 8 and
            len(data) == stream_offset + stream_bytes + (stream_bytes + 1023) // 1024,
            f'{name}: invalid DLT5 sections')
    checksum = 0
    for index, start in enumerate(range(stream_offset, stream_offset + stream_bytes, 1024)):
        for byte in data[start:min(start + 1024, stream_offset + stream_bytes)]:
            checksum = CRC8[checksum ^ byte]
        require(checksum == data[stream_offset + stream_bytes + index], f'{name}: stream CRC block {index}')
    if frames * 6 % 8:
        require(data[stream_offset + stream_bytes - 1] >> (frames * 6 % 8) == 0, f'{name}: nonzero final padding bits')
    return {'id': track['id'], 'kind': 'track', 'name': name, 'path': path,
            'metadata': [16000, frames, 2, 3], 'bytes': len(data), 'sha256': sha256(data),
            'crc32': crc32(data), 'frames': frames}


def validate_art(entry, art, track):
    name = track['name']
    require(entry['name'] == name and entry['id'] == track['id'], f'{name}: panorama/song pairing mismatch')
    path = art / entry['path']
    data = path.read_bytes()
    meta = document(path.with_suffix('.json'))
    width, height, padded_width, padded_height = entry['metadata']
    require(meta['format'] == 'havoc-panorama-v2' and meta['id'] == track['id'] and
            meta['logical_size'] == [width, height] and meta['storage_size'] == [padded_width, padded_height],
            f'{name}: panorama metadata mismatch')
    require(crc32((ROOT / 'assets/splashes' / meta['source']).read_bytes()) == int(meta['source_crc32'], 16),
            f'{name}: panorama source changed after baking')
    require(width >= 256 and height >= 256 and padded_width == (width + 15) // 16 * 16 and
            padded_height == (height + 7) // 8 * 8, f'{name}: unsupported panorama dimensions')
    require(len(data) == meta['payload_bytes'] == SEED_BYTES + padded_width * padded_height // 2 and
            crc32(data) == int(meta['payload_crc32'], 16), f'{name}: panorama extent/checksum mismatch')
    require(meta['tile_size'] == [16, 8] and meta['tile_bytes'] == 64 and
            meta['tile_offset'] == meta['seed_bytes'] == SEED_BYTES, f'{name}: wrong native tile format')
    origin = [((width - 256) // 2) & ~15, ((height - 256) // 2) & ~7]
    require(meta['seed_origin'] == origin, f'{name}: seed origin mismatch')
    # Check every seed byte against the independently addressed native tiles.
    tiles_y = padded_height // 8
    for column in range(128):
        world_column = origin[0] // 2 + column
        for row in range(256):
            world_y = origin[1] + row
            tile = (world_column // 8) * tiles_y + world_y // 8
            offset = SEED_BYTES + tile * 64 + (world_column % 8) * 8 + world_y % 8
            require(data[column * 256 + row] == data[offset], f'{name}: seed/native-tile disagreement')
    return {'id': track['id'], 'kind': 'panorama', 'name': name, 'path': path,
            'metadata': entry['metadata'], 'bytes': len(data), 'sha256': sha256(data),
            'crc32': crc32(data), 'seed_origin': origin}


def split_read(volumes, volume, offset, length):
    result = bytearray()
    while length:
        require(0 <= volume < len(volumes) and HEADER_BYTES <= offset < VOLUME_BYTES, 'invalid split asset range')
        count = min(length, VOLUME_BYTES - offset)
        result.extend(volumes[volume][offset:offset + count])
        length -= count
        volume += 1
        offset = HEADER_BYTES
    return bytes(result)


def verify_volumes(output, assets):
    first = (output / 'havocmd-1.reu').read_bytes()
    volume_count = first[13]
    require(1 <= volume_count <= 4, 'volume 1 must record one to four volumes')
    paths = [output / f'havocmd-{index + 1}.reu' for index in range(volume_count)]
    volumes = [first] + [path.read_bytes() for path in paths[1:]]
    require(not any((output / f'havocmd-{index + 1}.reu').exists() for index in range(volume_count, 4)),
            'stale volumes beyond the recorded count remain in the output folder')
    directory = volumes[0][64:HEADER_BYTES]
    expected_crc = crc32(directory)
    reports = []
    for index, (path, data) in enumerate(zip(paths, volumes)):
        require(len(data) == VOLUME_BYTES and data[:8] == b'HVMDVOL\0', f'{path.name}: bad size/magic')
        version, header, number, count, entries = struct.unpack_from('<HHBBH', data, 8)
        size, end, dcrc, pcrc, hcrc = struct.unpack_from('<IIIII', data, 16)
        require((version, header, number, count, entries, size) == (2, 4096, index, volume_count, 42, VOLUME_BYTES),
                f'{path.name}: wrong volume identity')
        require(data[64:HEADER_BYTES] == directory and dcrc == expected_crc, f'{path.name}: mismatched global directory')
        require(HEADER_BYTES <= end <= VOLUME_BYTES and crc32(data[HEADER_BYTES:end]) == pcrc,
                f'{path.name}: payload checksum mismatch')
        signed_header = bytearray(data[:HEADER_BYTES])
        signed_header[32:36] = bytes(4)
        require(crc32(signed_header) == hcrc, f'{path.name}: header checksum mismatch')
        reports.append({'file': path.name, 'bytes': len(data), 'sha256': sha256(data), 'payload_end': end})
    expected = {(asset['kind'], asset['id']): asset for asset in assets}
    packed_entries = []
    for index in range(42):
        record = directory[index * 64:(index + 1) * 64]
        identity, kind, volume, offset, length, checksum, *metadata = struct.unpack_from('<HBBIIIIIII', record)
        require(kind in (1, 2), 'unknown asset kind')
        label = 'track' if kind == 1 else 'panorama'
        asset = expected.pop((label, identity), None)
        require(asset is not None, 'duplicate or unexpected packed asset')
        require(record[32:].split(b'\0', 1)[0].decode('utf-8') == asset['name'] and
                metadata == asset['metadata'] and length == asset['bytes'] and checksum == asset['crc32'] and
                offset % ALIGNMENT == 0, f'{asset["name"]}: directory descriptor mismatch')
        packed = split_read(volumes, volume, offset, length)
        require(sha256(packed) == asset['sha256'] and crc32(packed) == checksum,
                f'{asset["name"]}: split payload differs from validated input')
        packed_entries.append({'id': identity, 'kind': label, 'name': asset['name'], 'volume': volume,
                               'offset': offset, 'length': length, 'crc32': f'{checksum:08x}',
                               'sha256': asset['sha256'], 'metadata': metadata})
    require(not expected and not any(directory[42 * 64:]), 'incomplete or nonzero unused directory')
    return expected_crc, reports, packed_entries


def host_path(path, packer):
    if os.name != 'nt' and packer.suffix.lower() == '.exe':
        return subprocess.check_output(['wslpath', '-w', str(path)], text=True).strip()
    return str(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog', type=Path, default=ROOT / 'assets/music/catalog.json')
    parser.add_argument('--music', type=Path, default=ROOT / 'target/music/full16')
    parser.add_argument('--art', type=Path, default=ROOT / 'target/art')
    parser.add_argument('--output', type=Path, default=ROOT / 'dist/havocmd')
    parser.add_argument('--header', type=Path, default=ROOT / 'target-c64/include/musicdisk_assets.h')
    parser.add_argument('--packer', type=Path, default=ROOT / 'target/release' / ('havocmd.exe' if os.name == 'nt' else 'havocmd'))
    args = parser.parse_args()
    args.music, args.art, args.output, args.header, args.packer = [p.resolve() for p in
        (args.music, args.art, args.output, args.header, args.packer)]
    catalog = document(args.catalog)
    require(catalog['profile'] == 'stereo16k3' and catalog['sample_rate'] == 16000 and catalog['channels'] == 2,
            'production catalog must be stereo16k3')
    tracks = catalog['tracks']
    by_name = {track['name']: track for track in tracks}
    require(len(tracks) == len(by_name) == 21 and {track['id'] for track in tracks} == set(range(21)), 'expected21 unique track IDs')
    playlist = catalog['playlist']
    require(len(playlist) == len(set(playlist)) == 21 and set(playlist) == set(by_name) and playlist[0] == 'hammond-rocker',
            'playlist must contain every complete song once, starting with Hammond')
    art_entries = document(args.art / 'assets.json')['assets']
    require(len(art_entries) == 21 and {entry['id'] for entry in art_entries} == set(range(21)), 'expected21 paired panoramas')
    by_id = {entry['id']: entry for entry in art_entries}
    assets = []
    for track in tracks:
        song = validate_track(track, args.music)
        picture = validate_art(by_id[track['id']], args.art, track)
        require(song['bytes'] <= SLOT_BYTES and song['bytes'] + picture['bytes'] <= SLOT_BYTES,
                f'{track["name"]}: song and panorama exceed the {SLOT_BYTES}-byte player slot')
        assets.extend((song, picture))
        print(f'Validated full song and panorama: {track["name"]}', flush=True)
    allocated = sum(align(asset['bytes']) for asset in assets)
    require(allocated <= 4 * PAYLOAD_BYTES, 'complete content exceeds four-volume capacity')
    require(args.packer.is_file(), 'build the Rust release packer before packaging')
    staging = ROOT / 'target/musicdisk-pack'
    staging.mkdir(parents=True, exist_ok=True)
    manifest = staging / 'assets.json'
    payload = {'format': 'havoc-asset-manifest-v2', 'assets': [
        {key: (Path(os.path.relpath(asset[key], staging)).as_posix() if key == 'path' else asset[key])
         for key in ('id', 'kind', 'name', 'path', 'metadata')} for asset in assets]}
    manifest.write_text(json.dumps(payload, indent=2) + '\n', encoding='utf-8')
    subprocess.run([str(args.packer), 'pack', host_path(manifest, args.packer), host_path(args.output, args.packer)], cwd=ROOT, check=True)
    subprocess.run([str(args.packer), 'check', host_path(args.output, args.packer)], cwd=ROOT, check=True)
    directory_crc, volumes, entries = verify_volumes(args.output, assets)
    ids = [by_name[name]['id'] for name in playlist]
    header = ('/* Generated after verified full-content packing; hardware acceptance is separate. */\n'
              '#ifndef HAVOC_MUSICDISK_ASSETS_H\n#define HAVOC_MUSICDISK_ASSETS_H\n#include <stdint.h>\n'
              '#define MUSICDISK_ASSETS_READY 1u\n#define MUSICDISK_FORMAT_VERSION 2u\n'
              f'#define MUSICDISK_DIRECTORY_CRC 0x{directory_crc:08x}UL\n'
              '#define MUSICDISK_SAMPLE_RATE 16000UL\n#define MUSICDISK_TRACK_COUNT 21u\n#define MUSICDISK_ASSET_COUNT 42u\n'
              f'static const uint8_t musicdisk_playlist[21] = {{{",".join(map(str, ids))}}};\n#endif\n')
    args.header.parent.mkdir(parents=True, exist_ok=True)
    args.header.write_text(header, encoding='utf-8')
    audio_bytes = sum(asset['bytes'] for asset in assets if asset['kind'] == 'track')
    art_bytes = sum(asset['bytes'] for asset in assets if asset['kind'] == 'panorama')
    report = {'format': 'havoc-musicdisk-bundle-v2', 'content_verified': True, 'hardware_verified': False,
              'profile': 'stereo16k3', 'sample_rate': 16000, 'channels': 2, 'output_bits_per_sample': 8,
              'full_song_count': 21, 'panorama_count': 21, 'playlist': playlist, 'playlist_ids': ids,
              'frames': sum(asset['frames'] for asset in assets if asset['kind'] == 'track'),
              'audio_bytes': audio_bytes, 'art_bytes': art_bytes, 'allocated_payload_bytes': allocated,
              'volume_count': len(volumes), 'volume_payload_bytes': len(volumes) * PAYLOAD_BYTES,
              'free_payload_bytes': len(volumes) * PAYLOAD_BYTES - allocated, 'directory_crc32': f'{directory_crc:08x}',
              'volumes': volumes, 'entries': entries,
              'panorama_format': '32768-byte seed;16x8 X-major tiles;8 packed-X columns x8 rows;even-X low nibble',
              'seed_origin': 'x=((width-256)//2)&~15; y=((height-256)//2)&~7'}
    (args.output / 'musicdisk-manifest.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    provenance = {'profile': 'stereo16k3', 'hardware_verified': False, 'tracks': [
        {'id': track['id'], 'name': track['name'],
         **document(args.music / (track['name'] + '.provenance.json'))} for track in tracks]}
    (args.output / 'music-provenance.json').write_text(json.dumps(provenance, indent=2) + '\n', encoding='utf-8')
    shutil.copyfile(ROOT / 'assets/music/LICENSE', args.output / 'MUSIC-LICENSE.txt')
    if (ROOT / 'LICENSE').is_file():
        shutil.copyfile(ROOT / 'LICENSE', args.output / 'LICENSE.txt')
    print(f'Verified42 assets in {len(volumes)} volumes; directoryCRC={directory_crc:08x}; payloadfree={report["free_payload_bytes"]}B; hardware unverified')


if __name__ == '__main__':
    main()
