// Screenshots page.html in Chromium (Playwright) at device scale 1 and
// writes the 240x40 strip of discs as a binary PPM (P6), the format the
// windowed test reads. Usage: node capture.mjs <playwright module dir> <out.ppm>
import { createRequire } from "node:module";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
const [pwDir, out] = process.argv.slice(2);
const require = createRequire(path.join(pwDir, "package.json"));
const { chromium } = require(pwDir);
const here = path.dirname(fileURLToPath(import.meta.url));
const W = 5 * 40 + 4 * 10, H = 40;
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 320, height: 80 }, deviceScaleFactor: 1 });
await page.goto("file://" + path.join(here, "page.html"));
// Raw pixels from a canvas the page decodes the PNG into: no PNG decoder here.
const png = await page.screenshot({ clip: { x: 0, y: 0, width: W, height: H } });
const rgba = await page.evaluate(async (b64) => {
  const img = new Image();
  img.src = "data:image/png;base64," + b64;
  await img.decode();
  const c = document.createElement("canvas");
  c.width = img.width; c.height = img.height;
  const ctx = c.getContext("2d");
  ctx.drawImage(img, 0, 0);
  return Array.from(ctx.getImageData(0, 0, img.width, img.height).data);
}, png.toString("base64"));
const header = Buffer.from(`P6\n${W} ${H}\n255\n`, "ascii");
const body = Buffer.alloc(W * H * 3);
for (let i = 0; i < W * H; i++) {
  body[3 * i] = rgba[4 * i]; body[3 * i + 1] = rgba[4 * i + 1]; body[3 * i + 2] = rgba[4 * i + 2];
}
writeFileSync(out, Buffer.concat([header, body]));
console.log(`${out}: ${W}x${H}, ${browser.version()}`);
await browser.close();
