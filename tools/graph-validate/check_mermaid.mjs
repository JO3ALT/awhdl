// Parse Mermaid files with the Mermaid library (no browser needed).
import { JSDOM } from 'jsdom';
import fs from 'fs';

const dom = new JSDOM('<!doctype html><html><body></body></html>');
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.DOMPurify = (await import('dompurify')).default(dom.window);
const mermaid = (await import('mermaid')).default;
mermaid.initialize({ startOnLoad: false });

let failed = 0;
for (const file of process.argv.slice(2)) {
  try {
    await mermaid.parse(fs.readFileSync(file, 'utf8'));
  } catch (error) {
    failed++;
    console.log('FAIL', file, String(error.message || error).slice(0, 400));
  }
}
console.log(`mermaid: ${process.argv.length - 2 - failed} ok, ${failed} failed`);
process.exit(failed ? 1 : 0);
