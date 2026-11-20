# Trying Zebra with huge and broken files

`scripts/make-test-media.sh` makes these files in `~/Downloads/zebra-test-media`. When the test is done, run `scripts/make-test-media.sh clean` to remove them all.

Disk use is small because most big files are sparse: their padding is a hole on APFS. The exception is `big/deck-with-…-video.pptx`, which really takes its full size (2 GB by default). For a different video size, use `ZEBRA_BIG_MB=6144 scripts/make-test-media.sh`.

## How to watch memory

Open Activity Monitor and find the Memory column for Zebra, or run `footprint Zebra` in Terminal. Note the figure with an empty document open, then compare it after each step.

**Pass:** memory goes up by tens of MB at most, never by the size of the file. Videos and big pictures are read from disk while they're needed, so they don't count against Zebra's memory.

Drawing a big picture on screen does cost memory for the pixels shown. Zebra draws pictures at screen size, so this stays small.

## Big files: each one should work

| # | Do this | Expect |
|---|---|---|
| 1 | Drag `big/video-2048MB.mp4` onto the canvas | A 6-second test pattern with a beep plays. Memory stays flat. |
| 2 | Drag `big/picture-217MB.png` onto the canvas | It appears (noise) after a moment. The window stays responsive, with no beachball for long. |
| 3 | Drag `big/picture-padded-1GB.png` onto the canvas, then `hostile/too-wide.png` | The first comes in: there is no size limit, as in Keynote. The second (a header claiming 2,000,000 pixels wide) is refused with a message that it can't be shown. Nothing is added for it. |
| 4 | File > Import `big/deck-with-2048MB-video.pptx` | The slides come in, and the video plays on its slide. Memory stays flat. |
| 5 | Drag `big/music-1024MB.m4a`, then `big/silence-1GB.wav` | Each becomes the background music. It plays when presenting. |
| 6 | Import `big/600-pages.pdf` | It asks first, or imports page by page (the over-200-pages prompt is coming). No freeze. |
| 7 | Save as `~/Downloads/zebra-test-media/test.zebra`, quit and reopen it | Everything is still there. The save is quick, because media is linked, not rewritten. |
| 8 | Delete the video, save, then Undo and save again | The video comes back. The file shrinks after the delete and grows again after the Undo. |
| 9 | Make an untitled document with the big video. Wait 1 minute (autosave), then force-quit Zebra from Activity Monitor and relaunch | The window comes back with the video. |
| 10 | Copy the big video, paste it into a second document, then close the first | The pasted video still plays. |
| 11 | File > Export > Web Page | The folder has the video as a file. Memory stays flat. |
| 12 | Paste a huge screenshot: take one with ⌘⇧⌃4 while zoomed in, or copy `picture-217MB.png` in Preview with ⌘A then ⌘C | It's pasted at full size, as in Keynote. The window doesn't hang. |

## Hostile files: each one should be refused, never crash

Drag each onto the canvas, or try File > Open / File > Import as the name suggests.

- `hostile/empty.png`, `hostile/empty.pptx`, `hostile/empty.zebra`
- `hostile/garbage.mp4`, `hostile/garbage.zebra`
- `hostile/text-named-picture.png`: a text file with a .png name
- `hostile/too-wide.png`: a PNG header claiming 2,000,000 pixels wide. Refused: it can't be scaled down to show.
- `hostile/picture-named-video.mov`: should be added as a picture, or refused; either is fine
- `hostile/cut-off-video.mp4`, `hostile/cut-off.pptx`, `hostile/lying-sizes.mp4`
- `hostile/disk-image-named.png`: claims to be 4 GB. Should be refused instantly, without reading 4 GB.
- `hostile/bomb-document.zebra`: 1 MB on disk, 1 GB unpacked. Opening it should say it's too large, and memory should not jump.
- `hostile/bomb-picture.zebra`: 2 MB on disk, 2 GB unpacked. It opens with a missing picture, and memory stays flat.
- `hostile/bomb-media.pptx`: its video unpacks to 2 GB. The import works or reports the video as broken, and memory stays flat.
- `hostile/pipe.png`: a named pipe. Should be refused at once, not hang.

**Pass:** Zebra never quits or beachballs. Each refused file gets a short message.

## Agent version (zebractl)

```sh
T=~/Downloads/zebra-test-media
zebractl new
zebractl apply "{\"op\":\"add_video\",\"path\":\"$T/big/video-2048MB.mp4\",\"x\":0,\"y\":0}"
zebractl add image "$T/big/picture-217MB.png" --x 2000 --y 0             # works
zebractl add image "$T/big/picture-padded-1GB.png" --x 0 --y 2000        # works: no size limit
zebractl add image "$T/hostile/too-wide.png" --x 0 --y 0                 # expect too_big
zebractl import "$T/big/deck-with-2048MB-video.pptx"
zebractl save "$T/test.zebra"
for f in "$T"/hostile/*; do zebractl add image "$f" --x 0 --y 0; done   # every one an error, app still up
zebractl open "$T/hostile/bomb-document.zebra" --force                  # expect bad_file
footprint Zebra | head -5
```

## Afterwards: nothing left behind

Quit Zebra, then run:

```sh
ls ~/Library/Caches/Zebra/media
ls ~/Library/Application\ Support/Zebra/Autosave ~/Library/Group\ Containers/*/Zebra/Autosave 2>/dev/null
ls -la ~/Downloads/zebra-test-media   # no .saving- / .open- / .part- files
```

The cache should be empty. Autosave should hold only documents still open or unsaved, plus an empty `Media` folder. Then remove the test files:

```sh
scripts/make-test-media.sh clean
```
