"""A slow link for testing Ranch: a TCP proxy that adds a delay each way.

    python3 -I scripts/latency-proxy.py LISTEN_PORT TARGET_PORT ONE_WAY_MS [--control FILE]

Every byte from the client reaches the target ONE_WAY_MS later, and every
reply comes back ONE_WAY_MS later: the round trip grows by 2 x ONE_WAY_MS.
Order is kept. With --control, the delay is read from FILE (a number of
milliseconds) on each chunk, so it can be changed while running:

    echo 150 > /tmp/delay   # 300 ms round trip from now on

See docs/slow-link.md.
"""

import argparse
import asyncio
import time


def parse():
    p = argparse.ArgumentParser(description="TCP proxy adding a one-way delay")
    p.add_argument("listen_port", type=int)
    p.add_argument("target_port", type=int)
    p.add_argument("one_way_ms", type=float)
    p.add_argument("--target-host", default="127.0.0.1")
    p.add_argument("--control", help="file holding the one-way delay in ms (read live)")
    return p.parse_args()


ARGS = parse()


def delay() -> float:
    if ARGS.control:
        try:
            with open(ARGS.control) as f:
                return float(f.read().strip()) / 1000
        except (OSError, ValueError):
            pass
    return ARGS.one_way_ms / 1000


async def pipe(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
    queue: asyncio.Queue = asyncio.Queue()

    async def receive():
        while True:
            data = await reader.read(65536)
            await queue.put((time.monotonic() + delay(), data))
            if not data:
                return

    async def send():
        while True:
            due, data = await queue.get()
            wait = due - time.monotonic()
            if wait > 0:
                await asyncio.sleep(wait)
            if not data:
                writer.close()
                return
            writer.write(data)
            await writer.drain()

    await asyncio.gather(receive(), send(), return_exceptions=True)


async def handle(client_reader, client_writer):
    target_reader, target_writer = await asyncio.open_connection(ARGS.target_host, ARGS.target_port)
    await asyncio.gather(
        pipe(client_reader, target_writer),
        pipe(target_reader, client_writer),
        return_exceptions=True,
    )


async def main():
    server = await asyncio.start_server(handle, "127.0.0.1", ARGS.listen_port)
    print(
        f"127.0.0.1:{ARGS.listen_port} -> {ARGS.target_host}:{ARGS.target_port}, "
        f"+{delay() * 1000:.0f} ms each way",
        flush=True,
    )
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
