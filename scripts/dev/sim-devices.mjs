#!/usr/bin/env node
// Simulates two accounts and several devices on this machine for agent
// delegation, without GitHub sign-in or a second computer.
//
// It runs the ignored `serve_simulated_devices` CLI test, which starts a
// Relay on 127.0.0.1 and real `openbitfun daemon run` targets (A2 answers,
// A3 holds until cancelled, A4 asks for approval, B1 belongs to another
// account), each with isolated storage and a mock model. Nothing is
// installed as a service, and every file lives under one temporary
// directory that is removed on exit.
//
//   node scripts/dev/sim-devices.mjs             start a desktop signed in as A1
//   node scripts/dev/sim-devices.mjs --check     run the desktop requester checks,
//                                                including delegation to account B
//                                                over its connections
//
// Options:
//   --copy-models   give A1 the `ai` section of your own config so it can run a
//                   real model (API keys are copied into the temporary
//                   directory, readable only by you, and deleted on exit)
//   --no-build      fail instead of building a missing desktop or frontend
//   --relay-port N  fix the Relay port (default: any free port)
//
// Closing the desktop keeps the devices running; press Enter (or send
// SIGUSR1) to start A1 again, for example to see a delegation picked up
// after a restart.
//
// Cargo and the desktop build use your rustup toolchain; set RUSTUP_TOOLCHAIN
// to choose another one.
import { spawn } from 'child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'fs';
import { homedir, platform, tmpdir } from 'os';
import { dirname, join, resolve } from 'path';
import { fileURLToPath } from 'url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const args = process.argv.slice(2);
const checkMode = args.includes('--check');
const copyModels = args.includes('--copy-models');
const noBuild = args.includes('--no-build');
const relayPort = optionValue('--relay-port') ?? '0';
const READY_TIMEOUT_MS = 30 * 60 * 1000;

function optionValue(name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}

function withLoopback(list) {
  return [list, '127.0.0.1', 'localhost'].filter(Boolean).join(',');
}

function log(message) {
  console.log(`[sim-devices] ${message}`);
}

function liveConfigPath() {
  const home = homedir();
  switch (platform()) {
    case 'darwin':
      return join(home, 'Library', 'Application Support', 'openbitfun', 'config', 'app.json');
    case 'win32':
      return join(process.env.APPDATA ?? join(home, 'AppData', 'Roaming'), 'openbitfun', 'config', 'app.json');
    default:
      return join(process.env.XDG_CONFIG_HOME ?? join(home, '.config'), 'openbitfun', 'config', 'app.json');
  }
}

function run(command, commandArgs, options = {}) {
  return new Promise((resolvePromise) => {
    const child = spawn(command, commandArgs, {
      cwd: repoRoot,
      stdio: 'inherit',
      windowsHide: true,
      ...options,
    });
    child.on('exit', (code, signal) => resolvePromise(signal ? 1 : code ?? 1));
    child.on('error', () => resolvePromise(1));
  });
}

// Asks `child` to exit (with `signal`, or already asked when null) and
// kills it if it has not exited within 15 seconds.
async function stopChild(child, signal) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((done) => child.once('exit', done));
  if (signal) child.kill(signal);
  const timedOut = new Promise((done) => setTimeout(done, 15000, 'timeout'));
  if ((await Promise.race([exited, timedOut])) === 'timeout') {
    child.kill('SIGKILL');
    await exited;
  }
}

async function waitForManifest(manifestPath, simulation) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  while (!existsSync(manifestPath)) {
    if (simulation.exitCode !== null || simulation.signalCode !== null) {
      throw new Error('the simulation stopped before it was ready');
    }
    if (Date.now() > deadline) {
      throw new Error('the simulation did not become ready in time');
    }
    await new Promise((done) => setTimeout(done, 500));
  }
  return JSON.parse(readFileSync(manifestPath, 'utf8'));
}

async function ensureDesktopBuilt() {
  const binary = join(repoRoot, 'target', 'debug', `openbitfun-desktop${platform() === 'win32' ? '.exe' : ''}`);
  const frontend = join(repoRoot, 'dist', 'index.html');
  if (existsSync(binary) && existsSync(frontend)) {
    return binary;
  }
  if (noBuild) {
    throw new Error(`missing ${existsSync(binary) ? frontend : binary}; run pnpm run desktop:build:fast`);
  }
  log('building the debug desktop and its frontend (pnpm run desktop:build:fast)');
  const code = await run('pnpm', ['run', 'desktop:build:fast'], { shell: platform() === 'win32' });
  if (code !== 0 || !existsSync(binary) || !existsSync(frontend)) {
    throw new Error('the desktop build failed');
  }
  return binary;
}

function describe(manifest) {
  const { devices } = manifest;
  log(`relay: ${manifest.relayUrl}`);
  log(`A1 (this desktop, account A): ${manifest.requester.deviceId}`);
  log(`A2 Laptop, answers at once:    remote:${devices.answers}/${manifest.openAgentId}`);
  log(`A3 Server, runs until cancelled: remote:${devices.holds}/${manifest.openAgentId}`);
  log(`A4 Workstation, asks approval: remote:${devices.asksApproval}/${manifest.openAgentId}`);
  log(`B1 Desktop, account B (not as a device of A): ${devices.otherAccount}`);
  log(`B1 over its connection, its owner approves: remote:conn:${manifest.connections.asksApproval.connectionId}/${manifest.openAgentId}`);
  log(`B2 Server over its connection, runs until stopped: remote:conn:${manifest.connections.holds.connectionId}/${manifest.openAgentId}`);
  log('the simulated devices run mock models, so their answers are test markers such as ACTIVE_TURN_STREAM_MARKER');
}

async function main() {
  const root = mkdtempSync(join(tmpdir(), 'openbitfun-sim-'));
  chmodSync(root, 0o700);
  const manifestPath = join(root, 'manifest.json');
  const requesterHome = join(root, 'a1', 'home');
  const requesterUserRoot = join(root, 'a1', 'user');
  mkdirSync(requesterHome, { recursive: true });
  mkdirSync(requesterUserRoot, { recursive: true });

  let desktop = null;
  let simulation = null;
  let exitCode = 0;
  let stopped = null;
  // Every caller (signal handler and normal exit) awaits the same cleanup,
  // so the process never exits before the temporary directory is removed.
  const stop = () => (stopped ??= stopEverything());
  const stopEverything = async () => {
    rmSync(manifestPath, { force: true });
    if (simulation && simulation.exitCode === null) {
      simulation.stdin.end();
    }
    // Wait for both to exit: the desktop keeps writing under `root` while it
    // shuts down, and the simulation stops its daemons as it exits.
    await Promise.all([stopChild(desktop, 'SIGTERM'), stopChild(simulation, null)]);
    rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  };
  // Backstop: the directory may hold copied API keys (--copy-models).
  process.on('exit', () => rmSync(root, { recursive: true, force: true }));
  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.on(signal, () => {
      stop().finally(() => process.exit(130));
    });
  }

  try {
    if (!checkMode) {
      // Build before the simulation starts: its devices should not idle
      // through a long desktop build.
      await ensureDesktopBuilt();
    }
    const simulationEnv = {
      ...process.env,
      OPENBITFUN_SIM_MANIFEST: manifestPath,
      OPENBITFUN_SIM_REQUESTER_HOME: requesterHome,
      OPENBITFUN_SIM_REQUESTER_USER_ROOT: requesterUserRoot,
      OPENBITFUN_SIM_RELAY_PORT: relayPort,
      OPENBITFUN_SIM_STOP_ON_STDIN_EOF: '1',
      NO_PROXY: withLoopback(process.env.NO_PROXY),
      no_proxy: withLoopback(process.env.no_proxy),
    };
    if (copyModels && !checkMode) {
      const source = liveConfigPath();
      if (!existsSync(source)) {
        throw new Error(`no config to copy models from at ${source}`);
      }
      simulationEnv.OPENBITFUN_SIM_REQUESTER_MODELS_FROM = source;
    }
    log('starting the local relay and simulated devices');
    simulation = spawn(
      'cargo',
      [
        'test', '--locked', '-p', 'openbitfun-cli', '--test', 'cli_command_contracts',
        '--', '--ignored', '--exact', 'delegation_simulation::serve_simulated_devices', '--nocapture',
      ],
      {
        cwd: repoRoot,
        env: simulationEnv,
        stdio: ['pipe', 'inherit', 'inherit'],
        windowsHide: true,
        // Ctrl+C reaches only this launcher, which then closes the
        // simulation's stdin so it drops (and stops) its daemons itself.
        detached: platform() !== 'win32',
      },
    );
    const manifest = await waitForManifest(manifestPath, simulation);
    describe(manifest);

    if (checkMode) {
      exitCode = await run(
        'cargo',
        [
          'test', '--locked', '-p', 'openbitfun-desktop', '--lib',
          'api::remote_agent_delegation::simulation_tests', '--', '--ignored', '--nocapture',
        ],
        {
          env: {
            ...process.env,
            OPENBITFUN_SIM_MANIFEST: manifestPath,
            // The check runs as A1: A1's records stay in
            // A1's simulated storage, never in this user's own.
            OPENBITFUN_E2E_STORAGE_GUARD: '1',
            OPENBITFUN_E2E_USER_ROOT: requesterUserRoot,
            OPENBITFUN_E2E_HOME: requesterHome,
            NO_PROXY: withLoopback(process.env.NO_PROXY),
            no_proxy: withLoopback(process.env.no_proxy),
          },
        },
      );
      log(exitCode === 0 ? 'desktop requester check passed' : 'desktop requester check failed');
    }

    if (checkMode) return;

    const binary = await ensureDesktopBuilt();
    log(`starting the desktop as A1 with storage under ${root}`);
    if (!copyModels) {
      log('A1 has no model configured; add one in Settings, or rerun with --copy-models');
    }
    log('ask the assistant to delegate to one of the remote: agents above; Ctrl+C stops everything');
    const desktopEnv = {
      ...process.env,
      OPENBITFUN_USER_ROOT: requesterUserRoot,
      OPENBITFUN_E2E_USER_ROOT: requesterUserRoot,
      OPENBITFUN_HOME: requesterHome,
      OPENBITFUN_E2E_HOME: requesterHome,
      OPENBITFUN_E2E_STORAGE_GUARD: '1',
      OPENBITFUN_E2E_PACKAGED_FRONTEND: '1',
      // The local Relay is on loopback; never route it through a proxy.
      NO_PROXY: withLoopback(process.env.NO_PROXY),
      no_proxy: withLoopback(process.env.no_proxy),
    };
    // Closing the desktop keeps the devices running, so a delegation can be
    // picked up again after A1 restarts (Enter, or SIGUSR1, starts it).
    const startDesktop = () => {
      if (desktop && desktop.exitCode === null && desktop.signalCode === null) return;
      desktop = spawn(binary, [], { cwd: repoRoot, stdio: 'inherit', windowsHide: true, env: desktopEnv });
      desktop.on('exit', () => {
        if (stopped) return;
        log('the A1 desktop exited; the devices keep running. Press Enter to start it again, Ctrl+C to stop');
      });
    };
    startDesktop();
    process.on('SIGUSR1', startDesktop);
    if (process.stdin.isTTY) {
      process.stdin.on('data', startDesktop);
    }
    exitCode = await new Promise((done) => {
      simulation.on('exit', () => {
        log('the simulation stopped');
        done(1);
      });
    });
  } catch (error) {
    console.error(`[sim-devices] ${error.message}`);
    exitCode = 1;
  } finally {
    await stop();
    // Also reached by the early return of --check, so its result is the
    // exit status.
    process.exit(exitCode);
  }
}

main();
