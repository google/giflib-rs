# giflib-rs

giflib-rs is an AI-assisted port of giflib to Rust. It is an API-compatible
drop-in replacement written in a memory safe language.

> [!NOTE]
> If API compatibility is not a strict requirement, we recommend using the
[gif](https://crates.io/crates/gif) crate instead. It has idiomatic
Rust APIs while being substantially faster.

## Safety

`giflib-rs` does not use `unsafe` in any of the business logic, but requires
`unsafe` to provide a C API and deal with pointers passed from C.
We use
[safer_cffi](https://crates.io/crates/safer_cffi)
to put up guardrails where possible.

## Known Differentials

 - **Setting `GifFile->Error` on invalid dimensions**: When an image descriptor
   has invalid dimensions (width or height ≤ 0, or width × height overflow),
   C's `DGifSlurp` returns `GIF_ERROR` but does **not** set `GifFile->Error`
   (leaving it at 0). Rust's `dgif_slurp` returns `GIF_ERROR` and sets
   `GifFile->Error = D_GIF_ERR_DATA_TOO_BIG` (108).  
   This is a deliberate change to remove what we consider a bug in C
   implementation.
 - **File Writing**: Rust checks the return code of `write()` calls and
   returns an error on failure. The C implementation silently ignores write
   failures and returns `GIF_OK`.  
   This is a deliberate change to improve over the C version.
 - **OOM Handling:** The Rust version panics on allocation error, whereas the C
   version returns an error code. This typically does not affect Linux-based
   systems, which overcommit memory and then use the OOM Killer instead of
   failing allocations.

-----------------

This is not an officially supported Google product. This project is not eligible
for the
[Google Open Source Software Vulnerability Rewards Program](https://bughunters.google.com/open-source-security).
