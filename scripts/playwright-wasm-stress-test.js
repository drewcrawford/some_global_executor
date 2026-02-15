const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

// Configuration
const NUM_RUNS = parseInt(process.env.NUM_RUNS || '10');
const WAIT_AFTER_LOAD_SECS = parseInt(process.env.WAIT_AFTER_LOAD || '30');
const TARGET_URL = process.env.TARGET_URL || 'http://127.0.0.1:1334';
const LOG_DIR = process.env.LOG_DIR || './stress_test_logs_wasm';
const LOAD_TIMEOUT_MS = 300000; // 5 minutes max to wait for load

// Ensure log directory exists
if (!fs.existsSync(LOG_DIR)) {
  fs.mkdirSync(LOG_DIR, { recursive: true });
}

const PANIC_LOG = path.join(LOG_DIR, 'panics_summary.txt');

// Clear previous panic log
if (fs.existsSync(PANIC_LOG)) {
  fs.unlinkSync(PANIC_LOG);
}

(async () => {
  console.log(`WASM Stress Test: ${NUM_RUNS} iterations`);
  console.log(`Wait after load: ${WAIT_AFTER_LOAD_SECS} seconds`);
  console.log(`Target URL: ${TARGET_URL}`);
  console.log(`Logs: ${LOG_DIR}/`);
  console.log('');

  let panicCount = 0;
  const failedRuns = [];
  const allLogs = [];

  const browser = await chromium.launch({
    headless: false,
    slowMo: 50
  });

  const context = await browser.newContext();
  const page = await context.newPage();

  // Collect console messages
  let currentRunLogs = [];
  page.on('console', msg => {
    const text = msg.text();
    currentRunLogs.push({ type: msg.type(), text });

    // Real-time panic detection
    if (text.toLowerCase().includes('panic')) {
      console.log(`  [PANIC DETECTED] ${text}`);
    }
  });

  // Collect page errors
  page.on('pageerror', error => {
    currentRunLogs.push({ type: 'error', text: error.message });
    console.log(`  [PAGE ERROR] ${error.message}`);
  });

  for (let i = 1; i <= NUM_RUNS; i++) {
    console.log(`=== Run ${i}/${NUM_RUNS} ===`);
    currentRunLogs = [];

    try {
      // Navigate or reload
      if (i === 1) {
        await page.goto(TARGET_URL, { waitUntil: 'domcontentloaded' });
      } else {
        await page.reload({ waitUntil: 'domcontentloaded' });
      }

      // Wait for loading complete message
      console.log('  Waiting for "Loading complete"...');

      let loadComplete = false;
      const startTime = Date.now();

      while (!loadComplete && (Date.now() - startTime) < LOAD_TIMEOUT_MS) {
        await page.waitForTimeout(1000);

        // Check if loading complete appeared in logs
        const hasLoadComplete = currentRunLogs.some(log =>
          log.text.includes('Loading complete - all systems are running')
        );

        if (hasLoadComplete) {
          loadComplete = true;
          console.log('  Loading complete detected!');
        }

        // Also check for early panic
        const hasPanic = currentRunLogs.some(log =>
          log.text.toLowerCase().includes('panic')
        );

        if (hasPanic) {
          break;
        }
      }

      if (!loadComplete) {
        const hasPanic = currentRunLogs.some(log =>
          log.text.toLowerCase().includes('panic')
        );
        if (!hasPanic) {
          console.log('  WARNING: Load timeout without panic');
        }
      }

      // Wait additional time after load (like --auto-terminate)
      if (loadComplete) {
        console.log(`  Waiting ${WAIT_AFTER_LOAD_SECS} seconds...`);
        await page.waitForTimeout(WAIT_AFTER_LOAD_SECS * 1000);
      }

      // Take screenshot
      const screenshotPath = path.join(LOG_DIR, `run_${i}.png`);
      await page.screenshot({ path: screenshotPath });
      console.log(`  Screenshot: run_${i}.png`);

      // Check for panics in this run's logs
      const runHasPanic = currentRunLogs.some(log =>
        log.text.toLowerCase().includes('panic')
      );

      // Save run log
      const runLogFile = path.join(LOG_DIR, `run_${i}.log`);
      const runLogContent = currentRunLogs.map(log => `[${log.type}] ${log.text}`).join('\n');
      fs.writeFileSync(runLogFile, runLogContent);

      if (runHasPanic) {
        console.log(`  PANIC DETECTED in run ${i}`);
        panicCount++;
        failedRuns.push(i);

        // Save panic details to summary
        const panicLogs = currentRunLogs.filter(log =>
          log.text.toLowerCase().includes('panic')
        );
        fs.appendFileSync(PANIC_LOG, `=== Run ${i} ===\n`);
        for (const log of panicLogs) {
          fs.appendFileSync(PANIC_LOG, `${log.text}\n`);
        }
        fs.appendFileSync(PANIC_LOG, '\n');

        allLogs.push({ run: i, status: 'panic', logs: panicLogs });
      } else if (!loadComplete) {
        console.log(`  TIMEOUT in run ${i}`);
        panicCount++;
        failedRuns.push(i);

        // Save timeout details to summary
        fs.appendFileSync(PANIC_LOG, `=== Run ${i} (timeout) ===\n`);
        const lastLogs = currentRunLogs.slice(-20);
        for (const log of lastLogs) {
          fs.appendFileSync(PANIC_LOG, `[${log.type}] ${log.text}\n`);
        }
        fs.appendFileSync(PANIC_LOG, '\n');

        allLogs.push({ run: i, status: 'timeout', logs: currentRunLogs.slice(-20) });
      } else {
        console.log(`  Run ${i} completed successfully`);
        allLogs.push({ run: i, status: 'success', logs: [] });
      }

    } catch (error) {
      console.log(`  ERROR in run ${i}: ${error.message}`);
      panicCount++;
      failedRuns.push(i);

      // Save error details
      const runLogFile = path.join(LOG_DIR, `run_${i}.log`);
      const runLogContent = currentRunLogs.map(log => `[${log.type}] ${log.text}`).join('\n');
      fs.writeFileSync(runLogFile, `ERROR: ${error.message}\n\n${runLogContent}`);

      fs.appendFileSync(PANIC_LOG, `=== Run ${i} (error) ===\n`);
      fs.appendFileSync(PANIC_LOG, `Error: ${error.message}\n\n`);

      allLogs.push({ run: i, status: 'error', error: error.message, logs: currentRunLogs });
    }

    console.log('');
  }

  await browser.close();

  // Summary
  console.log('=========================================');
  console.log('WASM Stress test complete!');
  console.log(`Total runs: ${NUM_RUNS}`);
  console.log(`Panics/failures detected: ${panicCount}`);

  if (panicCount > 0) {
    console.log(`Failed runs: ${failedRuns.join(', ')}`);
    console.log(`See ${PANIC_LOG} for details`);

    process.exit(1);
  } else {
    console.log('All runs completed successfully!');
    process.exit(0);
  }
})();
