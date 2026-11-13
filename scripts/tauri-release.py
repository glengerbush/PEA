#!/usr/bin/env python3
"""Adapt tauri-action's build command to packaging verified prebuilt inputs."""
import os
import subprocess
import sys
import time


def command(args, prebuilt):
    if prebuilt and args and args[0] == 'build':
        # Cargo's --locked does not apply to the bundler (which never compiles).
        if '--' in args:
            split = args.index('--')
            if args[split + 1:] != ['--locked']:
                raise ValueError('Unexpected Cargo options for prebuilt release')
            args = args[:split]
        return ['pnpm', 'tauri', 'bundle', *args[1:]]
    return ['pnpm', 'tauri', *args]


if __name__ == '__main__':
    prebuilt = os.environ.get('PEA_PREBUILT') == 'true'
    started = time.monotonic()
    result = subprocess.call(command(sys.argv[1:], prebuilt))
    elapsed = time.monotonic() - started
    mode = 'Verified inputs: bundle only' if prebuilt else 'Normal build and bundle'
    print(f'{mode}: {elapsed:.1f}s (exit {result})', flush=True)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as summary:
            summary.write(f'\n{mode}: **{elapsed:.1f}s**, exit {result}.\n')
    sys.exit(result)
