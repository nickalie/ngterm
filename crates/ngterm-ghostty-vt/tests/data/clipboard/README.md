These fixtures mirror Ghostty's `test/clipboard-multipart.py` at `LIBGHOSTTY_VT_REV`:
plain text, HTML, a 2×2 red RGB PNG, and bytes 0–255 repeated eight times.
`multipart.vt` sends all four in one OSC 5522 write transaction with the script's
program name and password.

The PNG uses zlib compression level 9 to keep its size at 73 bytes. The script's
default compression produces a different compressed length with some zlib
versions; its decoded pixels are the same. All layers compare the stored PNG
and arbitrary binary representation byte for byte.
