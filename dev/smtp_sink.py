#!/usr/bin/env python3
"""A throwaway SMTP server for development: accepts every mail and stores it under dev/data/mail/*.eml.

    python3 dev/smtp_sink.py [port]        # default 127.0.0.1:1025

In opennetmon (E-Mail settings): server 127.0.0.1, port 1025, encryption "none", no login.
Plain text only, no TLS, no authentication check. Never expose it to a network.
"""
import asyncio
import sys
import time
from pathlib import Path

OUT = Path(__file__).resolve().parent / "data" / "mail"


async def handle(reader, writer):
    async def say(line):
        writer.write(line.encode() + b"\r\n")
        await writer.drain()

    await say("220 opennetmon-dev-sink ESMTP")
    sender, recipients = "", []
    while True:
        raw = await reader.readline()
        if not raw:
            break
        line = raw.decode("utf-8", "replace").rstrip("\r\n")
        cmd = line.upper()
        if cmd.startswith(("EHLO", "HELO")):
            await say("250-opennetmon-dev-sink")
            await say("250 8BITMIME")
        elif cmd.startswith("MAIL FROM"):
            sender, recipients = line.split(":", 1)[1].strip(), []
            await say("250 ok")
        elif cmd.startswith("RCPT TO"):
            recipients.append(line.split(":", 1)[1].strip())
            await say("250 ok")
        elif cmd == "DATA":
            await say("354 end with <CRLF>.<CRLF>")
            lines = []
            while True:
                data = (await reader.readline()).decode("utf-8", "replace").rstrip("\r\n")
                if data == ".":
                    break
                lines.append(data[1:] if data.startswith("..") else data)
            OUT.mkdir(parents=True, exist_ok=True)
            name = OUT / f"{time.strftime('%Y%m%d-%H%M%S')}-{time.monotonic_ns() % 100000:05d}.eml"
            name.write_text(f"X-Envelope-From: {sender}\nX-Envelope-To: {', '.join(recipients)}\n" + "\n".join(lines) + "\n")
            subject = next((l for l in lines if l.lower().startswith("subject:")), "(no subject)")
            print(f"mail stored: {name.name}  {subject}", flush=True)
            await say("250 queued")
        elif cmd in ("RSET", "NOOP"):
            await say("250 ok")
        elif cmd == "QUIT":
            await say("221 bye")
            break
        else:
            await say("502 not implemented")
    writer.close()


async def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 1025
    server = await asyncio.start_server(handle, "127.0.0.1", port)
    print(f"SMTP sink listening on 127.0.0.1:{port}, writing to {OUT}", flush=True)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
