#!/usr/bin/env python3
"""Redact console identifiers while preserving boot progress and error text."""
import re


def redact_identifiers(text):
    text = re.sub(r'\b[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}\b',
                  '[UUID]', text)
    text = re.sub(r'\b(?:\d{1,3}\.){3}\d{1,3}\b', '[IP]', text)
    text = re.sub(r'\b(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}\b', '[MAC]', text)
    text = re.sub(r'(?i)\b(serial(?:\s+number)?|uuid|mac)\s*[:=]\s*\S+',
                  r'\1=[IDENTIFIER]', text)
    return text


def redact_secrets(text):
    text = re.sub(r'https?://\S+', '[URL]', text)
    text = re.sub(r'ssh-(?:ed25519|rsa)\s+\S+', '[SSH KEY]', text)
    text = re.sub(r'SHA256:[A-Za-z0-9+/=]+', '[KEY FINGERPRINT]', text)
    text = re.sub(r'(?i)\b(password|token|secret|api[_-]?key)\s*[:=]\s*\S+',
                  r'\1=[SECRET]', text)
    return text


def redact(text):
    return redact_secrets(redact_identifiers(text))
