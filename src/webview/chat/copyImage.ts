const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

export async function copyImage(src: string, mimeType: string): Promise<void> {
  // Pass the pending image to ClipboardItem before the user gesture expires while the blob loads.
  const png = fetch(src).then(async response => {
    if (!response.ok) throw new Error(`Image request failed: ${response.status}`);
    const blob = await response.blob();
    // Trust the bytes rather than the declared MIME type: the clipboard rejects a PNG entry that does not decode as PNG
    if (await isPng(blob)) return new Blob([blob], { type: 'image/png' });
    return toPng(await decode(blob, mimeType));
  });
  await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })]);
}

async function isPng(blob: Blob): Promise<boolean> {
  const head = new Uint8Array(await blob.slice(0, PNG_SIGNATURE.length).arrayBuffer());
  return PNG_SIGNATURE.every((byte, i) => head[i] === byte);
}

// Raster formats decode from the fetched bytes, which keeps the canvas untainted even though VS Code serves
// webview resources from another origin. createImageBitmap rejects SVG, so SVG goes through a data: URL <img>
// (the webview CSP allows data: images but not blob: ones).
async function decode(blob: Blob, mimeType: string): Promise<{ source: CanvasImageSource; width: number; height: number; close?: () => void }> {
  if (mimeType !== 'image/svg+xml' && blob.type !== 'image/svg+xml') {
    const bitmap = await createImageBitmap(blob);
    return { source: bitmap, width: bitmap.width, height: bitmap.height, close: () => bitmap.close() };
  }
  const image = new Image();
  image.src = await dataUrl(new Blob([blob], { type: 'image/svg+xml' }));
  await image.decode();
  return { source: image, width: image.naturalWidth, height: image.naturalHeight };
}

function dataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error ?? new Error('Image read failed'));
    reader.readAsDataURL(blob);
  });
}

async function toPng(image: { source: CanvasImageSource; width: number; height: number; close?: () => void }): Promise<Blob> {
  try {
    // An SVG without width / height or a viewBox has no intrinsic size and would paint nothing
    if (!image.width || !image.height) throw new Error('Image has no intrinsic size');
    const canvas = document.createElement('canvas');
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext('2d');
    if (!context) throw new Error('Canvas is unavailable');
    context.drawImage(image.source, 0, 0);
    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob(result => result ? resolve(result) : reject(new Error('Image conversion failed')), 'image/png');
    });
  } finally {
    image.close?.();
  }
}
