#!/usr/bin/env bash
# Install or update the private dump inbox on this VPS. Idempotent; run as root.
# The generated basic-auth password is written only to the 0600 credentials file.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
domain="${DUMP_INBOX_DOMAIN:-dumps.algosculptor.com}"
credentials=/root/.config/emulator-dumps/credentials
site=/etc/caddy/sites/emulator-dumps.caddy

id emudump >/dev/null 2>&1 || useradd --system --home-dir /srv/emulator-dumps --no-create-home --shell /usr/sbin/nologin emudump
install -d -m 0700 -o emudump -g emudump /srv/emulator-dumps
install -d -m 0755 /usr/local/lib/emulator-dumps
install -m 0644 "$repo/tools/dump_inbox.py" /usr/local/lib/emulator-dumps/dump_inbox.py
install -m 0644 "$repo/tools/dump_upload.py" /usr/local/lib/emulator-dumps/dump_upload.py
install -m 0644 "$repo/tools/systemd/emulator-dump-inbox.service" /etc/systemd/system/emulator-dump-inbox.service

if [[ ! -s "$credentials" ]]; then
	install -d -m 0700 "$(dirname "$credentials")"
	(umask 077; printf 'user=dump\npassword=%s\n' "$(python3 -c 'import secrets; print(secrets.token_urlsafe(24))')" >"$credentials")
fi
if [[ ! -s "$site" ]]; then
	hash="$(sed -n 's/^password=//p' "$credentials" | caddy hash-password)"
	(umask 077; cat >"$site" <<EOF
$domain {
	tls {
		issuer acme {
			disable_tlsalpn_challenge
		}
	}
	header {
		Strict-Transport-Security "max-age=31536000"
		X-Content-Type-Options nosniff
		X-Frame-Options DENY
		Referrer-Policy no-referrer
		X-Robots-Tag "noindex, nofollow"
	}
	request_body {
		max_size 70MB
	}
	basicauth {
		dump $hash
	}
	reverse_proxy 127.0.0.1:8791 {
		transport http {
			dial_timeout 5s
			response_header_timeout 120s
		}
	}
}
EOF
	)
	chown root:caddy "$site" 2>/dev/null && chmod 0640 "$site" || chmod 0644 "$site"
fi

caddy validate --config /etc/caddy/Caddyfile >/dev/null
systemctl daemon-reload
systemctl enable --now emulator-dump-inbox.service >/dev/null
systemctl restart emulator-dump-inbox.service
systemctl reload caddy
echo "dump inbox installed: https://$domain (credentials in $credentials)"
