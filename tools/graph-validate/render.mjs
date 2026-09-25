// Render .mmd files with Mermaid in headless Chrome and .dot files with the
// Graphviz WASM build, writing <file>.png next to each input.
import puppeteer from 'puppeteer';
import { Graphviz } from '@hpcc-js/wasm-graphviz';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const here = path.dirname(fileURLToPath(import.meta.url));
const graphviz = await Graphviz.load();
const browser = await puppeteer.launch({ args: ['--no-sandbox'] });
const page = await browser.newPage();
await page.setViewport({ width: 1800, height: 1200 });
let failed = 0;
for (const file of process.argv.slice(2)) {
  const text = fs.readFileSync(file, 'utf8');
  let svg;
  if (file.endsWith('.dot')) {
    svg = graphviz.dot(text);
  } else {
    await page.setContent('<html><body style="background:white"></body></html>');
    await page.addScriptTag({ path: path.join(here, 'node_modules/mermaid/dist/mermaid.min.js') });
    svg = await page.evaluate(async (source) => {
      mermaid.initialize({ startOnLoad: false, securityLevel: 'strict' });
      try { return (await mermaid.render('graph', source)).svg; }
      catch (error) { return 'ERROR ' + error.message; }
    }, text);
    if (svg.startsWith('ERROR')) { failed++; console.log('FAIL', file, svg); continue; }
  }
  await page.setContent(`<html><body style="margin:0;background:white">${svg}</body></html>`);
  const element = await page.$('svg');
  await element.screenshot({ path: file + '.png' });
  console.log('rendered', file + '.png');
}
await browser.close();
process.exit(failed ? 1 : 0);
