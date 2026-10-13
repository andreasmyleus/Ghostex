#!/usr/bin/env bash
# Create (or renew) the one Apple Development certificate the iOS TestFlight workflow reuses, and store
# it as the GitHub secrets APPLE_IOS_SIGNING_P12_BASE64 and APPLE_IOS_SIGNING_P12_PASSWORD.
#
#   tooling/release-mobile/ios-signing-certificate.sh
#
# Run it from a computer where `gh` is logged in with access to maddada/Ghostex (Git Bash on Windows
# works), once now and again before the certificate expires (a year; the TestFlight run warns 30 days
# ahead). The private key is made here, in a private temporary folder deleted on exit, and leaves
# only inside the p12 that `gh secret set` encrypts on this computer. GitHub receives the certificate
# request (a public key) and Apple's certificate (public).
#
# CDXC:Release 2026-10-11 DECISION:
# User: iOS builds reuse one long-lived signing certificate, like the user's Mac does, created once and stored in GitHub's encrypted secrets without the private key ever appearing unencrypted in the public repository's logs or artifacts; renewal is a one-command repeat. SEE-ALSO: .github/workflows/release-mobile-ios-signing-certificate.yml (creates the certificate with the API key), .github/workflows/release-mobile-ios-testflight.yml (imports it each run), asc-revoke-ci-certificates.mjs (never revokes it).
set -euo pipefail

REPO="maddada/Ghostex"
WORKFLOW="release-mobile-ios-signing-certificate.yml"
P12_SECRET="APPLE_IOS_SIGNING_P12_BASE64"
PASSWORD_SECRET="APPLE_IOS_SIGNING_P12_PASSWORD"
TEAM_ID="KTKP595G3B"

for command in gh openssl base64; do
	command -v "$command" >/dev/null || {
		echo "$command is required" >&2
		exit 1
	}
done
gh auth status >/dev/null

umask 077
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$WORK/key.pem" 2>/dev/null
# Git Bash would rewrite the -subj value's leading slash into a Windows path; only that argument is exempt.
MSYS2_ARG_CONV_EXCL='/CN=' openssl req -new -key "$WORK/key.pem" -subj "/CN=Ghostex iOS CI/C=US" -out "$WORK/request.csr"

REQUEST_ID="ios-signing-$(date -u +%Y%m%d%H%M%S)-$$"
echo "Asking GitHub Actions for the certificate ($REQUEST_ID)..."
gh workflow run "$WORKFLOW" -R "$REPO" --ref main \
	-f request_id="$REQUEST_ID" \
	-f csr_base64="$(base64 <"$WORK/request.csr" | tr -d '\r\n')"

RUN_ID=""
for _ in $(seq 1 60); do
	RUN_ID="$(gh run list -R "$REPO" --workflow "$WORKFLOW" --limit 20 --json databaseId,displayTitle \
		--jq ".[] | select(.displayTitle | endswith(\" $REQUEST_ID\")) | .databaseId" | head -n 1)"
	[[ -n "$RUN_ID" ]] && break
	sleep 5
done
[[ -n "$RUN_ID" ]] || {
	echo "The $WORKFLOW run for $REQUEST_ID did not appear within 5 minutes" >&2
	exit 1
}
echo "Run $RUN_ID: https://github.com/$REPO/actions/runs/$RUN_ID"
gh run watch "$RUN_ID" -R "$REPO" --exit-status >/dev/null || {
	echo "Run $RUN_ID failed; nothing was stored. See: gh run view $RUN_ID -R $REPO --log-failed" >&2
	exit 1
}
gh run download "$RUN_ID" -R "$REPO" -n "ios-signing-certificate-$REQUEST_ID" -D "$WORK/out"
CERT="$WORK/out/certificate.pem"

# The certificate must be Apple's answer to this key, for this team, with the CI name.
[[ "$(openssl x509 -in "$CERT" -noout -pubkey)" == "$(openssl pkey -in "$WORK/key.pem" -pubout)" ]] || {
	echo "The certificate does not belong to the generated key; nothing was stored" >&2
	exit 1
}
SUBJECT="$(openssl x509 -in "$CERT" -noout -subject -nameopt multiline)"
grep -Eq "^ +commonName += Apple Development: Created via API \([A-Z0-9]{10}\)$" <<<"$SUBJECT" &&
	grep -Eq "^ +organizationalUnitName += $TEAM_ID$" <<<"$SUBJECT" || {
	echo "Unexpected certificate subject; nothing was stored:" >&2
	echo "$SUBJECT" >&2
	exit 1
}

# 3DES and a SHA-1 MAC: the p12 encryption macOS `security import` reads on every runner image.
GHOSTEX_P12_PASSWORD="$(openssl rand -hex 32)"
export GHOSTEX_P12_PASSWORD
openssl pkcs12 -export -inkey "$WORK/key.pem" -in "$CERT" -name "Ghostex iOS CI signing" \
	-keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
	-passout env:GHOSTEX_P12_PASSWORD -out "$WORK/signing.p12"
openssl pkcs12 -in "$WORK/signing.p12" -passin env:GHOSTEX_P12_PASSWORD -noout

base64 <"$WORK/signing.p12" | tr -d '\r\n' | gh secret set "$P12_SECRET" -R "$REPO"
printf '%s' "$GHOSTEX_P12_PASSWORD" | gh secret set "$PASSWORD_SECRET" -R "$REPO"
unset GHOSTEX_P12_PASSWORD

echo "Stored $P12_SECRET and $PASSWORD_SECRET:"
cat "$WORK/out/certificate.json"
echo "Renew before it expires by running this script again; the TestFlight run warns 30 days ahead."
