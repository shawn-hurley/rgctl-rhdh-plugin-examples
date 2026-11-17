// Optional browser acceptance: use an installed Playwright and Chrome.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const fs = require('node:fs');
const assert = require('node:assert/strict');
async function main() {
  const browser = await chromium.launch({channel:'chrome',headless:true});
  const page = await browser.newPage({viewport:{width:1440,height:1080}});
  try {
    await page.goto('http://localhost:17007/catalog/default/api/quotes-api');
    const enter = page.getByRole('button',{name:'Enter',exact:true});
    await enter.waitFor({timeout:60000}); await enter.click(); await enter.waitFor({state:'hidden'});
    await page.getByRole('link',{name:'Catalog',exact:true}).waitFor({timeout:30000});
    await page.goto('http://localhost:17007/catalog/default/api/quotes-api');
    await page.getByText('quotes-api',{exact:true}).first().waitFor({timeout:30000});
    await page.getByRole('link',{name:'spring-quoters',exact:true}).first().waitFor();
    await page.getByRole('link',{name:'spring-quotes-client',exact:true}).first().waitFor();
    await page.screenshot({path:'local-dev/.generated/application-catalog.png',fullPage:true});
    const text = await page.locator('body').innerText();
    assert(text.includes('spring-quoters') && text.includes('spring-quotes-client'));
    fs.writeFileSync('local-dev/.generated/application-catalog-browser.json',JSON.stringify({status:'passed',url:page.url(),browser:await browser.version(),checks:['API entity visible','provider and consumer visible through built-in catalog relationships']},null,2)+'\n');
    console.log('The built-in API catalog page shows both the provider and consumer.');
  } catch(error) {
    await page.screenshot({path:'local-dev/.generated/application-catalog-failure.png',fullPage:true});
    fs.writeFileSync('local-dev/.generated/application-catalog-failure.txt',await page.locator('body').innerText());
    throw error;
  } finally { await browser.close(); }
}
main().catch(error=>{console.error(error);process.exitCode=1;});
