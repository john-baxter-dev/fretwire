// Files on *this* side of the seam. Under Tauri the backend and the UI share a disk, so a native
// picker hands the backend a path and the backend does the I/O. In a browser — serve mode, or the
// mock — the daemon's disk is somewhere else (a Pi across the room), so the file's bytes travel in
// the invoke instead: `<input type="file">` in, an anchor download out, base64 in between for the
// binary case (an IR is a few KB, so that is cheap). The `_inline` command variants take and
// return these.

/// Open a file chooser. Resolves to the `File`, or `null` if the user cancelled.
export function pickFile({ accept } = {}) {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    if (accept) input.accept = accept;
    input.style.display = "none";
    let done = false;
    const finish = (file) => {
      if (done) return;
      done = true;
      input.remove();
      resolve(file);
    };
    input.addEventListener("change", () => finish(input.files?.[0] ?? null));
    // Fired by current browsers when the dialog is dismissed. Older ones never settle here, which
    // only means a cancelled picker leaves a dangling promise — nothing awaits it for long.
    input.addEventListener("cancel", () => finish(null));
    document.body.appendChild(input);
    input.click();
  });
}

/// Hand the browser a file to save, under `name`.
export function saveFile(name, blob) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  // Revoked after the click has been dispatched; revoking synchronously races the download in
  // some browsers.
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

/// Standard base64 of raw bytes. Chunked: `String.fromCharCode(...bytes)` overflows the argument
/// list well below an IR's size on some engines.
export function bytesToBase64(bytes) {
  const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let bin = "";
  for (let i = 0; i < u8.length; i += 0x8000) {
    bin += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
  }
  return btoa(bin);
}

export function base64ToBytes(b64) {
  const bin = atob(b64);
  const u8 = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) u8[i] = bin.charCodeAt(i);
  return u8;
}

/// A file's name without its extension — what an IR upload is called by default.
export const fileStem = (name) => name.replace(/\.[^.]+$/, "");
