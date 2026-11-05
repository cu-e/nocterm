`composer-noise.png` is generated test data, with no external image source. Its
256 × 256 RGBA pixels are consecutive SHA-256 outputs for
`b"nocterm-composer-transport-noise-v1" + counter.to_bytes(4, "big")`, with counter
0 through 8191. Each PNG scanline uses filter 0; the raster is compressed with
Python's standard-library `zlib.compress(rows, 6)`. The PNG has IHDR, IDAT and IEND
chunks, with 8-bit RGBA, no interlace and standard CRC32 checksums.

The image exceeds 128 KiB so the live composer regression must cross the relay's
16 KiB frame boundary while preserving the original PNG bytes and selected model.
