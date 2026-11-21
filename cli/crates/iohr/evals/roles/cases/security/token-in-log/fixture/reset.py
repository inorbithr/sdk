"""Password reset: issue a one-time link."""

import logging
import secrets

log = logging.getLogger("accounts.reset")


def start_reset(email, outbox):
    """Make a reset token for email and queue the link. Returns the token."""
    token = secrets.token_urlsafe(32)
    log.info("reset requested for %s with token %s", email, token)
    outbox.append((email, f"https://example.com/reset?t={token}"))
    return token
