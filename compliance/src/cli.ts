#!/usr/bin/env node

import { CrossTester, CrossTestConfig } from './cross-test.js';
import { join } from 'path';
import { access } from 'fs/promises';

async function crossTestSDKs(sdks: string[]): Promise<void> {
  const config: CrossTestConfig = {
    sdks,
    scenariosPath: join(process.cwd(), 'scenarios/data.json'),
    goldensPath: join(process.cwd(), 'scenarios/goldens'),
    resultsPath: join(process.cwd(), 'test-results'),
    timeout: 10000 // 10 seconds
  };

  const tester = new CrossTester(config);

  console.log('🔍 Validating SDKs...');
  for (const sdk of sdks) {
    const isValid = await tester.validateSDK(sdk);
    if (!isValid) {
      console.error(`❌ SDK ${sdk} not found or missing binaries`);
      process.exit(1);
    }
    console.log(`✅ SDK ${sdk} validated`);
  }

  console.log('\n📋 Loading scenarios...');
  await tester.loadScenarios();

  console.log('🧮 Generating test matrix...');
  const testMatrix = tester.generateTestMatrix();
  console.log(`Generated ${testMatrix.length} test combinations`);

  console.log('\n🚀 Running tests...');
  const results = [];
  let completed = 0;

  for (const test of testMatrix) {
    try {
      const result = await tester.runTest(test);
      results.push(result);
      completed++;

      const status = result.success ? '✅' : '❌';
      console.log(`${status} [${completed}/${testMatrix.length}] ${test.clientSDK}->${test.serverSDK} scenario ${test.scenarioId}`);
    } catch (error) {
      console.error(`💥 Test failed: ${error}`);
      completed++;
    }
  }

  console.log('\n📊 Generating report...');
  await tester.generateReport(results);

  const failedCount = results.filter(r => !r.success).length;
  process.exit(failedCount > 0 ? 1 : 0);
}

// CLI entry point
if (import.meta.url === `file://${process.argv[1]}`) {
  const args = process.argv.slice(2);

  if (args.length === 0) {
    console.error('Usage: cross-test <sdk1> [sdk2 ...]');
    process.exit(1);
  }

  crossTestSDKs(args).catch(error => {
    console.error('Error:', error);
    process.exit(1);
  });
}

export { crossTestSDKs };