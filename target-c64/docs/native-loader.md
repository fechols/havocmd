# Native REU volume loader

`disk_init()` runs before the panorama cache is initialized. It probes the
current Ultimate DOS directory, then configured Home, then the actual root
directory entries. For each mounted directory it tries `/mount/havocmd/`.
Consequently names such as `USB11` are discovered, rather than guessed.
`COPY_UI_PATH` is not used: the firmware does not implement it.

The chosen folder must contain `havocmd-1.reu` through `havocmd-4.reu`, each
exactly 16 MiB. All four 4096-byte headers must pass their header CRC, compiled
directory CRC, format/index/count checks, metadata checks, and asset range
checks. The first verified directory is copied to REU0. Runtime destination
ranges never overlap this directory. No preloaded REU image is required.

Two 5 MiB bundle slots begin at REU `$001000` and `$501000`. A bundle contains the
compressed audio followed immediately by its panorama payload. The returned
`DiskBundle` gives both addresses. The panorama starts with its 32768-byte seed;
native tiles follow. `disk_pin(slot)` and `disk_release(slot)` count playback
and visual-transition owners independently. A pinned slot cannot be selected
for loading, and an in-flight request cannot be replaced by another request.

`disk_step()` sends at most 16 command FIFO bytes or drains at most 32 reply FIFO
bytes per call. The caller supplies the audio deadline guard. Each native
`LOAD_REU` operation transfers at most 16 KiB and also stops at 64 KiB boundaries.
At a 16 MiB file boundary the loader closes the file, opens the next volume, and
seeks past its 4096-byte header. This is a file read into REU, not a C64 reset or
a complete REU image replacement.

Every transfer must return success and the exact requested byte count and REU
destination. A successful DOS status alone is insufficient because a short
EOF read can report success. The firmware's space-padded hexadecimal reply is
validated. Payload asset CRCs are recorded in the directory but are not read
back over the live audio bus; startup validates headers and metadata, while
runtime validates each transfer's status/count/destination.

A timeout latches an error but does not cancel native I/O. Its destination
remains reserved until the eventual reply is drained and acknowledged. The
loader then closes the file and invalidates only the incoming bundle. Pinned
playback data remains intact. Retrying is possible after `disk_busy` clears.
If firmware never completes the request, the slot deliberately stays reserved.

The register client only issues DOS read/open/close/seek/path/directory commands
and FIFO acknowledgments. It does not change firmware configuration, reset
the C64, use a network service, or write a file. The protocol follows the
[Ultimate DOS implementation](https://github.com/GideonZ/1541ultimate/blob/master/software/filemanager/dos.cc)
and its `LOAD_REU` command 0x21.

Local verification:

```sh
cc -std=c99 -O1 -g -Wall -Wextra -Werror -fsanitize=address,undefined \
  tools/test-disk-loader.c -o /tmp/havoc-disk-test
/tmp/havoc-disk-test
```

The model covers discovered `USB11` paths, all four headers, valid-CRC malformed
metadata, file boundaries, two pinned slots, short reads, delayed completion,
timeout ownership, retry, and bounded FIFO work. It is not a hardware timing
measurement. Physical Ultimate validation remains a separate gate.
