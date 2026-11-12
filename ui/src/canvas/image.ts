// Pictures for the canvas: a pasted or dropped image is scaled down to at most 2400 px on its long
// side and re-encoded as WebP (quality 0.85), so one screenshot stays small enough for the canvas
// file (the browser keeps boards in localStorage, about 5 MB) and its `src` stays under the
// schema's cap. The picture is stored in the canvas item itself; nothing else holds it.

import { IMAGE_SRC_RE, MAX_IMAGE_SRC } from "./schema.ts";

export const MAX_SIDE = 2400;
export const QUALITY = 0.85;

/** The size an image is stored at: the long side at most `max`, the shape kept, never below 1 px. */
export function fitSize(w: number, h: number, max = MAX_SIDE): { w: number; h: number } {
  const k = Math.min(1, max / Math.max(w, h, 1));
  return { w: Math.max(1, Math.round(w * k)), h: Math.max(1, Math.round(h * k)) };
}

/** The size an image is first shown at on the board: its natural size, at most `box` on the long side. */
export function shownSize(w: number, h: number, box = { w: 640, h: 480 }): { w: number; h: number } {
  const k = Math.min(1, box.w / Math.max(w, 1), box.h / Math.max(h, 1));
  return { w: Math.max(1, Math.round(w * k)), h: Math.max(1, Math.round(h * k)) };
}

/** True for a file the canvas can take: a PNG, WebP or JPEG (the schema's three). */
export const isCanvasImage = (f: { type: string }) => /^image\/(png|webp|jpeg)$/.test(f.type);

/** The image files in a paste or a drop, in order. */
export function imageFiles(list: FileList | readonly File[] | null | undefined): File[] {
  return Array.from(list ?? []).filter(isCanvasImage);
}

const toDataUrl = (b: Blob) =>
  new Promise<string>((ok, no) => {
    const r = new FileReader();
    r.onload = () => ok(String(r.result));
    r.onerror = () => no(r.error);
    r.readAsDataURL(b);
  });

const toBlob = (c: HTMLCanvasElement, type: string, q: number) =>
  new Promise<Blob | null>((ok) => c.toBlob(ok, type, q));

/**
 * Scales an image file down and re-encodes it. Returns the data URL (WebP, or PNG where the browser
 * can't write WebP), what it was stored at and its natural size; null when it can't be read or
 * stays over the cap even at lower quality.
 */
export async function encodeImage(
  file: Blob,
): Promise<{ src: string; w: number; h: number; natural_w: number; natural_h: number } | null> {
  let bmp: ImageBitmap;
  try {
    bmp = await createImageBitmap(file);
  } catch {
    return null;
  }
  try {
    let { w, h } = fitSize(bmp.width, bmp.height);
    for (let attempt = 0; attempt < 4; attempt++) {
      const cv = document.createElement("canvas");
      cv.width = w;
      cv.height = h;
      const cx = cv.getContext("2d");
      if (!cx) return null;
      cx.imageSmoothingQuality = "high";
      cx.drawImage(bmp, 0, 0, w, h);
      const blob = await toBlob(cv, "image/webp", QUALITY - attempt * 0.1);
      if (!blob) return null;
      const src = await toDataUrl(blob);
      if (IMAGE_SRC_RE.test(src) && src.length <= MAX_IMAGE_SRC)
        return { src, w, h, natural_w: bmp.width, natural_h: bmp.height };
      ({ w, h } = fitSize(w, h, Math.round(Math.max(w, h) * 0.75)));
    }
    return null;
  } finally {
    bmp.close();
  }
}
