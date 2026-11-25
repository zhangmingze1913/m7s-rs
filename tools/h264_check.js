const path = require('path');
const puppeteer = require('puppeteer-core');

(async () => {
  const chrome = path.resolve('tools/chrome/chrome-win64/chrome.exe');
  const browser = await puppeteer.launch({
    executablePath: chrome,
    userDataDir: path.resolve('chrome-h264-' + process.pid),
    headless: 'new',
    args: ['--no-sandbox', '--disable-gpu'],
  });
  const page = await browser.newPage();
  await page.goto('about:blank');
  const result = await page.evaluate(() => {
    const ms = MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E"');
    const caps = RTCRtpReceiver.getCapabilities('audio');
    const codecs = (caps?.codecs || []).map((c) => c.mimeType);
    return { h264_ms: ms, codecs };
  });
  console.log(JSON.stringify(result));
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
