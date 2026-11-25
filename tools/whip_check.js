const path = require('path');
const puppeteer = require('puppeteer-core');

(async () => {
  const browser = await puppeteer.launch({
    executablePath: path.resolve('tools/chrome/chrome-win64/chrome.exe'),
    userDataDir: path.resolve('chrome-whip-' + process.pid),
    headless: 'new',
    args: ['--no-sandbox', '--disable-gpu', '--disable-web-security'],
  });
  const page = await browser.newPage();
  page.on('console', (m) => console.log('CONSOLE', m.text()));
  await page.goto('file:///' + path.resolve('tools/whip_test.html').replace(/\\/g, '/'), { waitUntil: 'domcontentloaded' });
  try {
    await page.waitForFunction(() => document.getElementById('out').textContent.includes('PUSHED'), { timeout: 30000 });
  } catch (e) {
    console.log('WAIT_TIMEOUT');
  }
  console.log(await page.$eval('#out', (e) => e.textContent));
  const streams = await page.evaluate(async () => {
    const r = await fetch('http://127.0.0.1:8180/api/streams');
    return await r.text();
  });
  console.log('API', streams);
  await browser.close();
})().catch((e) => { console.error('FAIL', e); process.exit(1); });
