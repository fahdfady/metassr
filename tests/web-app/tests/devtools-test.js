/**
 * Playwright (Chromium) tests for the MetaSSR devtools panel.
 *
 * Requires a running dev server for tests/web-app, e.g.:
 *   metassr dev --port 4310 --ws-port 4311
 *
 * Run with:
 *   CHROMIUM_PATH=... node tests/devtools-test.js
 */

const fs = require('fs');
const path = require('path');
const { chromium } = require('playwright');

const BASE_URL = process.env.METASSR_TEST_URL || 'http://localhost:4310';

let browser;
let passed = 0;
let failed = 0;

function assert(condition, message) {
    if (condition) {
        console.log(`  [PASS] ${message}`);
        passed++;
    } else {
        console.error(`  [FAIL] ${message}`);
        failed++;
    }
}

async function fetchJson(page, url, options) {
    return page.evaluate(
        async ([requestUrl, requestOptions]) => {
            const response = await fetch(requestUrl, requestOptions);
            return { status: response.status, body: await response.json() };
        },
        [url, options]
    );
}

async function waitForRoute(page, route, options = {}) {
    const locator = page
        .locator(`#metassr-events .event[data-route="${route}"]`)
        .first();
    await locator.waitFor({ timeout: options.timeout || 15000 });
    return locator;
}

async function waitForSystemRow(page, needle, options = {}) {
    const locator = page.locator('#metassr-events .system').filter({ hasText: needle });
    await locator.first().waitFor({ timeout: options.timeout || 30000 });
    return locator;
}

async function waitForFetchContains(page, url, needle, timeout = 30000) {
    await page.waitForFunction(
        async ([requestUrl, expected]) => {
            const response = await fetch(requestUrl, { cache: 'no-store' });
            const text = await response.text();
            return text.includes(expected);
        },
        [url, needle],
        { timeout }
    );
}

async function testPanelOpens() {
    console.log('\n[Suite] Panel injection and toggle');
    const page = await browser.newPage();
    await page.goto(BASE_URL);

    const devtoolsScripts = await page
        .locator('script[src="/__metassr/devtools.js"]')
        .count();
    assert(devtoolsScripts === 1, `devtools script injected exactly once (got ${devtoolsScripts})`);

    const liveReloadScripts = await page
        .locator('script[src="/livereload/script.js"]')
        .count();
    assert(liveReloadScripts === 1, `live-reload script injected exactly once (got ${liveReloadScripts})`);

    const badge = page.locator('#metassr-badge');
    await badge.waitFor({ timeout: 10000 });
    assert(await badge.isVisible(), 'DevTools badge is visible');

    const panel = page.locator('#metassr-panel');
    assert(!(await panel.isVisible()), 'Panel is hidden initially');

    await badge.click();
    assert(await panel.isVisible(), 'Panel opens when badge is clicked');

    await page.close();
}

async function testApiTracing() {
    console.log('\n[Suite] API route tracing');
    const page = await browser.newPage();
    await page.goto(BASE_URL);
    await page.locator('#metassr-badge').click();
    await page.locator('#metassr-panel').waitFor();

    // GET (node)
    const getResult = await fetchJson(page, '/api/hello');
    assert(getResult.status === 200, `GET /api/hello returns 200 (got ${getResult.status})`);

    const getRow = await waitForRoute(page, '/api/hello');
    assert(
        (await getRow.locator('.method').innerText()).includes('GET'),
        'GET trace row shows GET method'
    );
    assert(
        (await getRow.locator('.status').innerText()).includes('200'),
        'GET trace row shows 200 status'
    );
    assert(
        (await getRow.locator('.loader').innerText()).toLowerCase() === 'node',
        'GET trace row shows node loader'
    );
    assert(
        /\d+(\.\d+)?ms/.test(await getRow.locator('.duration').innerText()),
        'GET trace row shows a duration'
    );

    // Detail view
    await getRow.click();
    const detail = page.locator('#metassr-detail');
    await detail.waitFor({ timeout: 5000 });
    const detailText = (await detail.innerText()).toLowerCase();
    assert(detailText.includes('get /api/hello'), 'Detail view shows method and route');
    assert(detailText.includes('request'), 'Detail view has a Request section');
    assert(detailText.includes('response'), 'Detail view has a Response section');
    assert(
        detailText.includes('hello from metassr api!'),
        'Detail view shows the response body'
    );

    // POST (node)
    const postResult = await fetchJson(page, '/api/hello', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'DevTools' }),
    });
    assert(postResult.status === 201, `POST /api/hello returns 201 (got ${postResult.status})`);

    const postRow = await waitForRoute(page, '/api/hello');
    assert(
        (await postRow.locator('.method').innerText()).includes('POST'),
        'POST trace row shows POST method'
    );
    assert(
        (await postRow.locator('.status').innerText()).includes('201'),
        'POST trace row shows 201 status'
    );

    // GET (python loader)
    const pyResult = await fetchJson(page, '/api/pygreet');
    assert(pyResult.status === 200, `GET /api/pygreet returns 200 (got ${pyResult.status})`);

    const pyRow = await waitForRoute(page, '/api/pygreet');
    assert(
        (await pyRow.locator('.loader').innerText()).toLowerCase() === 'python',
        'Python route trace row shows python loader'
    );

    const badgeCount = await page.locator('#metassr-count').innerText();
    assert(Number(badgeCount) >= 3, `Badge counts traced calls (got ${badgeCount})`);

    await page.close();
}

async function testApiHotReload() {
    console.log('\n[Suite] API route hot-reload');
    const page = await browser.newPage();
    await page.goto(BASE_URL);
    await page.locator('#metassr-badge').click();
    await page.locator('#metassr-panel').waitFor();

    const apiPath = path.join(__dirname, '..', 'src', 'api', 'hello.js');
    const original = fs.readFileSync(apiPath, 'utf8');

    try {
        fs.writeFileSync(
            apiPath,
            original.replace('Hello from MetaSSR API!', 'Hello from the hot reload!')
        );

        await waitForSystemRow(page, 'API route reloaded: src/api/hello.js', { timeout: 30000 });
        assert(true, 'Panel reports the API route reload');

        await waitForFetchContains(page, '/api/hello', 'Hello from the hot reload!');
        assert(true, 'API route executes the new code without restarting the server');

        const reloadedRow = await waitForRoute(page, '/api/hello');
        assert(
            (await reloadedRow.locator('.status').innerText()).includes('200'),
            'Traced call after hot reload succeeds'
        );
    } finally {
        fs.writeFileSync(apiPath, original);
    }

    await waitForFetchContains(page, '/api/hello', 'Hello from MetaSSR API!');
    assert(true, 'API route restored after the test');

    await page.close();
}

async function testPageHotReload() {
    console.log('\n[Suite] SSR page hot-reload');
    const page = await browser.newPage();
    await page.goto(BASE_URL);

    const pagePath = path.join(__dirname, '..', 'src', 'pages', 'home.tsx');
    const original = fs.readFileSync(pagePath, 'utf8');

    try {
        fs.writeFileSync(
            pagePath,
            original.replace(
                'This is a simple home page contains a counter',
                'This is the hot reloaded home page'
            )
        );

        await waitForFetchContains(page, '/home', 'This is the hot reloaded home page', 60000);
        assert(true, 'SSR output reflects the rebuilt page without restarting the server');
    } finally {
        fs.writeFileSync(pagePath, original);
    }

    await waitForFetchContains(page, '/home', 'This is a simple home page contains a counter', 60000);
    assert(true, 'Page restored after the test');

    await page.close();
}

async function testDevSourcemaps() {
    console.log('\n[Suite] Dev sourcemaps');
    const page = await browser.newPage();

    const response = await page.request.get(`${BASE_URL}/dist/pages/index.js.js.map`);
    assert(response.ok(), `Page bundle sourcemap is served (got ${response.status()})`);

    const map = await response.json();
    assert(Array.isArray(map.sources) && map.sources.length > 0, 'Sourcemap has sources');
    assert(
        map.sources.some((source) => source.includes('src/pages/index.tsx')),
        'Sourcemap maps back to the original page source'
    );

    await page.close();
}

async function runTests() {
    console.log('Launching Chromium...');
    browser = await chromium.launch({
        executablePath: process.env.CHROMIUM_PATH || undefined,
    });

    await testPanelOpens();
    await testApiTracing();
    await testApiHotReload();
    await testPageHotReload();
    await testDevSourcemaps();

    await browser.close();

    console.log(`\n=== Results: ${passed} passed, ${failed} failed ===`);
    if (failed > 0) process.exit(1);
}

runTests().catch((err) => {
    console.error('Unexpected error:', err);
    if (browser) browser.close();
    process.exit(1);
});
