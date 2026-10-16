#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///

import os
from pathlib import Path
import select
import sys
import termios
import time
import tty


def write(data):
    view = memoryview(data)
    while view:
        view = view[os.write(1, view):]


def sync():
    write(b'\x1b[6n')
    reply = b''
    deadline = time.monotonic() + 20
    while not reply.endswith(b'R'):
        timeout = max(0, deadline - time.monotonic())
        if not select.select([0], [], [], timeout)[0]:
            raise TimeoutError('training terminal stopped responding')
        chunk = os.read(0, 4096)
        if not chunk:
            raise EOFError('training terminal closed')
        reply += chunk


old = termios.tcgetattr(0)
tty.setraw(0)
try:
    write(b'\x1b]0;Cryptty PGO training\x07')
    time.sleep(2)
    sync()
    for variant in range(6):
        lines = []
        for i in range(512):
            text = (
                f'{i:04d} INFO service worker request '
                f'completed in {i % 97}ms'
            )
            if variant == 1:
                text = f'\x1b[{30 + i % 8}m{text}\x1b[0m'
            elif variant == 2:
                text = f'{i} 文档与终端 café e\u0301 世界 😀 编译完成'
            elif variant == 3:
                text = f'\x1b[2K\r{text}\x1b[12Dupdated\x1b[K'
            elif variant == 4:
                text = (
                    f'\x1b]8;;https://example.com/{i}\x1b\\'
                    f'{text}\x1b]8;;\x1b\\'
                )
            elif variant == 5:
                text = f'\x1b[38;2;{i % 256};120;{i // 2}m{text}\x1b[0m'
            lines.append(text + '\r\n')
        chunk = ''.join(lines).encode()
        for _ in range(256):
            write(chunk)
        sync()
    for i in range(90):
        write(
            f'\x1b[H\x1b[2J\x1b[1;3;4mStatus {i}\x1b[0m\r\n'.encode()
        )
        write('文件导航  █░▒▓\r\n'.encode())
        write(b'\x1b[4;20Heditor cursor\x1b[K')
        sync()
        time.sleep(1 / 60)
finally:
    termios.tcsetattr(0, termios.TCSANOW, old)

Path(sys.argv[1]).write_text('ok\n')
