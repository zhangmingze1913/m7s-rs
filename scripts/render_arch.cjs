const path = require('path');
const puppeteer = require('puppeteer-core');

const executablePath = process.env.CHROME_HEADLESS_SHELL || path.resolve('chrome-headless-shell/win64-158.0.8127.36/chrome-headless-shell-win64/chrome-headless-shell.exe');
const html = path.resolve(process.argv[2]);
const png = path.resolve(process.argv[3]);

(async () => {
  const browser = await puppeteer.launch({
    executablePath,
    headless: 'new',
    args: ['--no-sandbox', '--disable-gpu', '--hide-scrollbars'],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1800, height: 900, deviceScaleFactor: 1.5 });
  await page.goto('file:///' + html.replace(/\\/g, '/'), { waitUntil: 'load' });
  const height = await page.evaluate(() => Math.ceil(document.body.scrollHeight));
  await page.setViewport({ width: 1800, height, deviceScaleFactor: 1.5 });
  await page.screenshot({ path: png, clip: { x: 0, y: 0, width: 1800, height } });
  await browser.close();
  console.log(png);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
