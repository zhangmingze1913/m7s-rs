const path = require('path');
const puppeteer = require('puppeteer-core');

(async () => {
  const chrome = path.resolve('tools/chrome/chrome-win64/chrome.exe');
  const pageUrl = 'file:///' + path.resolve('tools/whep_av_split.html').replace(/\\/g, '/');
  const browser = await puppeteer.launch({
    executablePath: chrome,
    userDataDir: path.resolve("chrome-profile-" + process.pid),
    headless: 'new',
    args: [
      '--no-sandbox',
      '--disable-gpu',
      '--disable-web-security',
      '--autoplay-policy=no-user-gesture-required',
    ],
  });
  const page = await browser.newPage();
  page.on('console', (m) => console.log('CONSOLE', m.text()));
  page.on('pageerror', (e) => console.log('PAGEERROR', e.message));
  await page.goto(pageUrl, { waitUntil: 'domcontentloaded' });
  try {
    await page.waitForFunction(
      () => document.getElementById('out').textContent.includes('FINAL'),
      { timeout: 30000 }
    );
  } catch (e) {
    console.log('WAIT_TIMEOUT');
  }
  console.log(await page.$eval('#out', (e) => e.textContent));
  await browser.close();
})().catch((e) => {
  console.error('FAIL', e);
  process.exit(1);
});
