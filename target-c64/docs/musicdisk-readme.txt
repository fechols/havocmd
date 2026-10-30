HAVOC MUSICDISK FOR C64 ULTIMATE
Production candidate - physical hardware validation pending

The C64 player uses Rust for playback, streaming and camera control, with
6502 assembly for the timed raster, audio, panorama copy and timer kernels.

CONTENTS
All 21 complete songs: 16 kHz true stereo, signed 8-bit playback.
Total duration before the playlist wraps: 34 minutes 37.8835 seconds.
Every song has its own full 453 x 543 logical panorama, with 16 colors.
The renderer targets 200 x 240 double-width logical pixels across the C64's
usual borders. Physical HDMI edge coverage and combined timing are pending.
Panoramas stay level and scroll smoothly in two dimensions. The automatic
camera reverses at the artwork edges; song changes slide in the next picture.

INSTALL AND START
Put the complete bundle in a folder named havocmd at the root of your SD card
or USB drive. Keep havocmd.prg, every havocmd-N.reu volume (currently two), this file,
havocmd.cfg, the manifests and licenses together.

Select NTSC and HDMI output. Apply havocmd.cfg, then run havocmd.prg.
Do not preload a REU image. The REU files are storage volumes; the program
finds its current folder, configured Home or /havocmd/ on an actual mounted
device and loads the needed ranges through the Ultimate Command Interface.
All volumes must match the PRG's compiled directory checksum.

The config enables 64 MHz software-controlled turbo, 16 MB REU, Ultimate Audio
and the Command Interface. Badline timing and REU preload are disabled.
Automatic configuration saving is off.

Startup stays black while the first song and picture load, the silent sampler
rate is measured, and at least 3,968 stereo frames are prepared. Picture and music then
start together. During playback the next song and panorama load in the
background; the current pair stays active until the replacement is ready.

CONTROLS
F1: previous song.
F3: next song.
Space: next song, once per press.
F5: resume automatic panning.
F7: mute/unmute; the song and playback clock continue.
Joystick port 2: pan manually. Fire: resume automatic panning.
Reset the C64U to exit.

PLAYLIST
Hammond Rocker, Lowland Approach, Crystal Cave, Iron Canyon, Canyon Gate,
Ashen Wastes, Foundry Belt, Fortress Run, Citadel Core, Storm Front,
Meteor Reach, Carrier Coast, Carrier Assault, Asteroid Belt, Lunar Awe,
Ship Graveyard, Orbital Shipyard, Space Carrier, Jump Gate, Mission Failed,
Galactic Dash, then Hammond Rocker again.

The Shattered Moon image is paired with Lunar Awe. Every recording plays to
its original end; natural silence and gaps in the source recordings remain.
The player decodes compressed DLT5 music in real time. It adds no deliberate
pause or loading screen between prepared songs. Published audio stays about a
quarter second ahead of the sampler. The outgoing song fades out over its final
second; a manual skip ends the current song one second after the key press, so
the next one begins as the fade reaches silence. Picture changes use a
progressive transition while audio continues: the camera keeps scrolling
through the fade and pauses only for its final quarter second, while the next
picture's seed loads, then slides into the new picture as the new song starts.

STARTUP FAILURE COLORS
Persistent red: storage/UCI failure, missing volume or mismatched directory.
Persistent purple: Ultimate Audio is unavailable or not enabled.
Persistent orange: invalid song/picture data or sampler calibration failure.
Correct the configuration or restore the matching bundle, then reset and run
again. A temporary black startup screen is normal.

VALIDATION STATUS
The build checks all content and its checksums before finalizing a bundle. The
manifests contain volume and asset SHA-256 hashes and recording provenance.
The linked-program model runs at least 600 frames with automatic scrolling,
checking exact stereo output, rendered pixels, frame cadence and cache readiness.
It also checks that the first picture is level and that startup copies the
resident code before its original storage becomes the image cache.
The 16 kHz profile reduces the decoder and transfer work per frame, leaving more
time for scrolling and background loading. Host tests are not a physical timing
pass. This production candidate still needs a C64U
run that checks startup, continuous stereo, every picture edge, scrolling,
background loading and song transitions. No current hardware pass is claimed.

Earlier two-second prototypes and their preload/control instructions are
historical diagnostics, not validation of this full production bundle.

Music and artwork: William Echols. See MUSIC-LICENSE.txt and LICENSE.txt.
