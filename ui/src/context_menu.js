export function isDevMode() {
  return window.__WISP_DEV__ === true;
}

export async function copyImage(src) {
  const source = await (await fetch(src)).blob();
  let png = source;
  if (source.type !== "image/png") {
    const bitmap = await createImageBitmap(source);
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    canvas.getContext("2d").drawImage(bitmap, 0, 0);
    bitmap.close();
    png = await new Promise((resolve, reject) => canvas.toBlob(
      (blob) => blob ? resolve(blob) : reject(new Error("Could not encode image")),
      "image/png",
    ));
  }
  await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
}
