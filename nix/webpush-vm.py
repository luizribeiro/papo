import base64
import json
import os
import re
import shlex
import struct

from cryptography.hazmat.primitives import hashes, hmac, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDFExpand


def b64url(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode("ascii")


private_key = ec.generate_private_key(ec.SECP256R1())
public_key = private_key.public_key().public_bytes(
    serialization.Encoding.X962,
    serialization.PublicFormat.UncompressedPoint,
)
auth_secret = os.urandom(16)
p256dh = b64url(public_key)
auth = b64url(auth_secret)


def run(*args):
    command = " ".join(shlex.quote(value) for value in (client, *args))
    return ergo.succeed(command)


def records():
    output = receiver.succeed(
        "test -f /var/lib/webpush-receiver/requests.jsonl && "
        "cat /var/lib/webpush-receiver/requests.jsonl"
    )
    return [json.loads(line) for line in output.splitlines()]


def wait_records(count):
    receiver.wait_until_succeeds(
        "test -f /var/lib/webpush-receiver/requests.jsonl && "
        f"test $(wc -l < /var/lib/webpush-receiver/requests.jsonl) -eq {count}"
    )
    return records()


def extract(key, value):
    digest = hmac.HMAC(key, hashes.SHA256())
    digest.update(value)
    return digest.finalize()


def expand(key, info, length):
    return HKDFExpand(
        algorithm=hashes.SHA256(), length=length, info=info
    ).derive(key)


def decrypt(record, expected_path):
    headers = record["headers"]
    assert record["path"] == expected_path
    assert headers["content-encoding"] == "aes128gcm"
    assert headers["urgency"] == "high"
    assert "authorization" in headers
    body = base64.b64decode(record["body"])
    salt = body[:16]
    record_size = struct.unpack(">I", body[16:20])[0]
    key_length = body[20]
    server_public_bytes = body[21 : 21 + key_length]
    ciphertext = body[21 + key_length :]
    assert record_size == 2048
    assert key_length == 65
    assert len(body) == 2048
    assert len(ciphertext) == 1962

    server_public = ec.EllipticCurvePublicKey.from_encoded_point(
        ec.SECP256R1(), server_public_bytes
    )
    shared_secret = private_key.exchange(ec.ECDH(), server_public)
    key_prk = extract(auth_secret, shared_secret)
    key_info = b"WebPush: info\0" + public_key + server_public_bytes
    ikm = expand(key_prk, key_info, 32)
    prk = extract(salt, ikm)
    content_key = expand(prk, b"Content-Encoding: aes128gcm\0", 16)
    nonce = expand(prk, b"Content-Encoding: nonce\0", 12)
    plaintext = AESGCM(content_key).decrypt(nonce, ciphertext, None).rstrip(b"\0")
    assert plaintext.endswith(b"\x02")
    return plaintext[:-1].decode("utf-8")


def assert_message(line, target, text):
    tags, message = line.split(" ", 1)
    assert tags.startswith("@")
    parsed_tags = dict(tag.split("=", 1) for tag in tags[1:].split(";"))
    assert re.fullmatch(r"[A-Za-z0-9]+", parsed_tags["msgid"])
    assert re.fullmatch(r"\d{4}-\d{2}-\d{2}T.*Z", parsed_tags["time"])
    pattern = rf":sender![^ ]+ PRIVMSG {re.escape(target)} :{re.escape(text)}"
    assert re.fullmatch(pattern, message), line
