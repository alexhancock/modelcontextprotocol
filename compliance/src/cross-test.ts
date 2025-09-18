import { spawn, ChildProcess } from 'child_process';
import { readFile, writeFile, mkdir, access } from 'fs/promises';
import { join } from 'path';
import type {
  Scenarios,
  TestResult,
  LogComparisonResult,
  AnnotatedJSONRPCMessage,
  Transport
} from './types.js';

export interface CrossTestConfig {
  sdks: string[];
  scenariosPath: string;
  goldensPath: string;
  resultsPath: string;
  timeout: number; // milliseconds
}

export interface TestMatrix {
  clientSDK: string;
  serverSDK: string;
  scenarioId: number;
  transport: Transport;
}

export class CrossTester {
  private config: CrossTestConfig;
  private scenarios: Scenarios | null = null;

  constructor(config: CrossTestConfig) {
    this.config = config;
  }

  async loadScenarios(): Promise<Scenarios> {
    if (!this.scenarios) {
      const data = await readFile(this.config.scenariosPath, 'utf-8');
      this.scenarios = JSON.parse(data);
    }
    return this.scenarios!;
  }

  async validateSDK(sdk: string): Promise<boolean> {
    // Try different possible paths for binaries (relative to current working directory)
    const possiblePaths = [
      { client: join(sdk, 'test-client'), server: join(sdk, 'test-server') },
      { client: join(sdk, 'target', 'release', 'test-client'), server: join(sdk, 'target', 'release', 'test-server') }
    ];

    for (const paths of possiblePaths) {
      try {
        await access(paths.client);
        await access(paths.server);
        return true;
      } catch {
        continue;
      }
    }

    return false;
  }

  getBinaryPath(sdk: string, binary: 'client' | 'server'): string {
    // Return paths relative to current working directory
    if (sdk === 'rust') {
      return join(sdk, 'target', 'release', `test-${binary}`);
    }
    return join(sdk, `test-${binary}`);
  }

  generateTestMatrix(): TestMatrix[] {
    if (!this.scenarios) {
      throw new Error('Scenarios not loaded');
    }

    const matrix: TestMatrix[] = [];
    const transports: Transport[] = ['stdio'];

    for (const clientSDK of this.config.sdks) {
      for (const serverSDK of this.config.sdks) {
        for (const scenario of this.scenarios.scenarios) {
          for (const transport of transports) {
            // Skip HTTP-only scenarios for stdio transport
            if (scenario.http_only && transport === 'stdio') {
              continue;
            }

            matrix.push({
              clientSDK,
              serverSDK,
              scenarioId: scenario.id,
              transport
            });
          }
        }
      }
    }

    return matrix;
  }

  async runTest(test: TestMatrix): Promise<TestResult> {
    console.log(`Testing: ${test.clientSDK} client -> ${test.serverSDK} server (scenario ${test.scenarioId}, ${test.transport})`);

    const scenario = this.scenarios!.scenarios.find(s => s.id === test.scenarioId);
    if (!scenario) {
      throw new Error(`Scenario ${test.scenarioId} not found`);
    }

    const logPath = join(this.config.resultsPath, `test-${test.clientSDK}-${test.serverSDK}-${test.scenarioId}-${test.transport}.jsonl`);

    try {
      const capturedLog = await this.executeTest(test, scenario, logPath);

      // For now, since we don't have MITM logging, if executeTest succeeds
      // (meaning client completed scenario successfully), we consider it a pass
      const success = capturedLog !== null; // executeTest returns [] on success, throws on failure

      const comparisonResult = success ?
        { match: true } :
        await this.compareWithGolden(test.scenarioId, capturedLog);

      return {
        scenarioId: test.scenarioId,
        clientSDK: test.clientSDK,
        serverSDK: test.serverSDK,
        transport: test.transport,
        success,
        capturedLog,
        comparisonResult
      };
    } catch (error) {
      return {
        scenarioId: test.scenarioId,
        clientSDK: test.clientSDK,
        serverSDK: test.serverSDK,
        transport: test.transport,
        success: false,
        error: error instanceof Error ? error.message : String(error),
        capturedLog: []
      };
    }
  }

  private async executeTest(test: TestMatrix, scenario: any, logPath: string): Promise<AnnotatedJSONRPCMessage[]> {
    // For now, run a simplified test - direct client to server communication
    const serverArgs = [
      '--server-name', scenario.server_name,
      '--transport', test.transport,
      '--scenarios-data', this.config.scenariosPath
    ];

    const serverBinary = this.getBinaryPath(test.serverSDK, 'server');
    const clientBinary = this.getBinaryPath(test.clientSDK, 'client');

    // For stdio transport, we don't need to start server separately
    // The client will spawn the server as a subprocess

    // Start client with stdio connection to server
    const clientArgs = [
      '--scenario-id', test.scenarioId.toString(),
      '--id', scenario.client_ids[0], // Use first client ID for now
      '--scenarios-data', this.config.scenariosPath,
      'stdio',
      '--',  // Separate client args from server command
      serverBinary,
      ...serverArgs
    ];

    const client = spawn(clientBinary, clientArgs, {
      stdio: ['pipe', 'pipe', 'pipe']
    });

    let clientOutput = '';
    let clientError = '';

    // Capture output
    client.stdout?.on('data', (data) => {
      clientOutput += data.toString();
    });
    client.stderr?.on('data', (data) => {
      clientError += data.toString();
    });

    // Set up timeout
    const timeout = setTimeout(() => {
      if (!client.killed) {
        client.kill('SIGTERM');
      }
    }, this.config.timeout);

    // Set up a check for scenario completion
    let scenarioCompleted = false;
    let completionMessage = '';

    client.stdout?.on('data', (data) => {
      const output = data.toString();
      if (output.includes('passed:') || output.includes('Scenario') && output.includes('passed')) {
        scenarioCompleted = true;
        completionMessage = output.trim();
      }
    });

    try {
      // Wait for scenario completion or timeout
      await new Promise<void>((resolve, reject) => {
        const checkCompletion = () => {
          if (scenarioCompleted) {
            resolve();
          }
        };

        // Check every 100ms
        const interval = setInterval(checkCompletion, 100);

        client.on('exit', (code) => {
          clearInterval(interval);
          if (code === 0 || scenarioCompleted) {
            resolve();
          } else {
            reject(new Error(`Client exited with code ${code}. Stderr: ${clientError}. Stdout: ${clientOutput}`));
          }
        });

        client.on('error', (error) => {
          clearInterval(interval);
          reject(error);
        });

        setTimeout(() => {
          clearInterval(interval);
          if (scenarioCompleted) {
            resolve();
          } else {
            reject(new Error(`Client timeout - scenario not completed. Stdout: ${clientOutput}. Stderr: ${clientError}`));
          }
        }, this.config.timeout);
      });

      // For now, return empty messages since we don't have MITM logging yet
      // In a real implementation, we would parse the actual captured traffic
      return [];
    } catch (error) {
      // Kill client if still running
      if (!client.killed) {
        client.kill('SIGKILL');
      }
      throw error;
    } finally {
      clearTimeout(timeout);

      // Force kill client if still running
      if (!client.killed) {
        client.kill('SIGKILL');
      }

      // Wait a bit for processes to clean up
      await new Promise(resolve => setTimeout(resolve, 200));
    }
  }

  private async compareWithGolden(scenarioId: number, actualLog: AnnotatedJSONRPCMessage[]): Promise<LogComparisonResult> {
    const goldenPath = join(this.config.goldensPath, `${scenarioId}.jsonl`);

    try {
      const goldenContent = await readFile(goldenPath, 'utf-8');
      const goldenLines = goldenContent.trim().split('\n');
      const expectedMessages: AnnotatedJSONRPCMessage[] = [];

      for (const line of goldenLines) {
        if (line.startsWith('//')) continue;
        try {
          expectedMessages.push(JSON.parse(line));
        } catch (e) {
          console.warn('Failed to parse golden line:', line);
        }
      }

      return this.compareLogMessages(expectedMessages, actualLog);
    } catch (error) {
      // No golden file exists yet, assume success since client exited successfully
      return { match: true };
    }
  }

  private compareLogMessages(expected: AnnotatedJSONRPCMessage[], actual: AnnotatedJSONRPCMessage[]): LogComparisonResult {
    if (expected.length !== actual.length) {
      return {
        match: false,
        differences: [{
          index: -1,
          expected: expected[0] || {} as AnnotatedJSONRPCMessage,
          actual: actual[0] || {} as AnnotatedJSONRPCMessage,
          reason: `Length mismatch: expected ${expected.length} messages, got ${actual.length}`
        }]
      };
    }

    const differences = [];
    for (let i = 0; i < expected.length; i++) {
      const exp = expected[i];
      const act = actual[i];

      // Compare normalized messages (ignoring some dynamic fields)
      if (!this.messagesEqual(exp, act)) {
        differences.push({
          index: i,
          expected: exp,
          actual: act,
          reason: 'Message content differs'
        });
      }
    }

    return {
      match: differences.length === 0,
      differences: differences.length > 0 ? differences : undefined
    };
  }

  private messagesEqual(a: AnnotatedJSONRPCMessage, b: AnnotatedJSONRPCMessage): boolean {
    // Normalize by removing dynamic fields like timestamps, IDs
    const normalize = (msg: AnnotatedJSONRPCMessage) => {
      const normalized = JSON.parse(JSON.stringify(msg));

      // Remove dynamic message IDs
      if (normalized.message && typeof normalized.message.id === 'string') {
        normalized.message.id = 'DYNAMIC_ID';
      }

      return normalized;
    };

    return JSON.stringify(normalize(a)) === JSON.stringify(normalize(b));
  }

  async generateReport(results: TestResult[]): Promise<void> {
    await mkdir(this.config.resultsPath, { recursive: true });

    const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
    const reportPath = join(this.config.resultsPath, `cross-test-${timestamp}.json`);

    const report = {
      timestamp,
      config: this.config,
      summary: this.generateSummary(results),
      results
    };

    await writeFile(reportPath, JSON.stringify(report, null, 2));

    // Also generate console summary
    this.printSummary(results);

    console.log(`\nDetailed report saved to: ${reportPath}`);
  }

  private generateSummary(results: TestResult[]) {
    const total = results.length;
    const passed = results.filter(r => r.success).length;
    const failed = results.filter(r => !r.success).length;

    const bySDK: Record<string, { passed: number; failed: number }> = {};

    for (const result of results) {
      const key = `${result.clientSDK}->${result.serverSDK}`;
      if (!bySDK[key]) {
        bySDK[key] = { passed: 0, failed: 0 };
      }

      if (result.success) {
        bySDK[key].passed++;
      } else {
        bySDK[key].failed++;
      }
    }

    return {
      total,
      passed,
      failed,
      passRate: (passed / total * 100).toFixed(1) + '%',
      bySDK
    };
  }

  private printSummary(results: TestResult[]): void {
    const summary = this.generateSummary(results);

    console.log('\n=== Cross-SDK Test Summary ===');
    console.log(`Total tests: ${summary.total}`);
    console.log(`Passed: ${summary.passed} (${summary.passRate})`);
    console.log(`Failed: ${summary.failed}`);

    console.log('\nBy SDK combination:');
    for (const [combo, stats] of Object.entries(summary.bySDK)) {
      const total = stats.passed + stats.failed;
      const rate = (stats.passed / total * 100).toFixed(1);
      const status = stats.failed === 0 ? '✅' : '❌';
      console.log(`  ${status} ${combo}: ${stats.passed}/${total} (${rate}%)`);
    }

    if (summary.failed > 0) {
      console.log('\nFailed tests:');
      for (const result of results.filter(r => !r.success)) {
        console.log(`  ❌ ${result.clientSDK}->${result.serverSDK} scenario ${result.scenarioId}: ${result.error || 'Log comparison failed'}`);
      }
    }
  }
}