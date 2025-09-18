#!/usr/bin/env node

import { CrossTester, CrossTestConfig } from './cross-test.js';
import { join } from 'path';

async function testSingle(): Promise<void> {
  const config: CrossTestConfig = {
    sdks: ['rust'],
    scenariosPath: join(process.cwd(), 'scenarios/data.json'),
    goldensPath: join(process.cwd(), 'scenarios/goldens'),
    resultsPath: join(process.cwd(), 'test-results'),
    timeout: 5000 // 5 seconds
  };

  const tester = new CrossTester(config);
  await tester.loadScenarios();

  // Test just scenario 1
  const testCase = {
    clientSDK: 'rust',
    serverSDK: 'rust',
    scenarioId: 1,
    transport: 'stdio' as const
  };

  console.log(`Testing scenario ${testCase.scenarioId}...`);
  try {
    const result = await tester.runTest(testCase);
    console.log('Test result:', JSON.stringify(result, null, 2));
    if (result.success) {
      console.log('✅ Test passed!');
    } else {
      console.log('❌ Test failed:', result.error);
    }
  } catch (error) {
    console.log('💥 Test threw exception:', error);
  }
}

testSingle().catch(console.error);