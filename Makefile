setup:
	[ -f .env.local ] || cp .env.example .env.local
	npm install

run:
	npm run dev

# Against a hub running on this machine (see deskconn-web-app's dev-local).
run-local:
	VITE_WAMP_WT_URL=https://localhost:8082/wamp \
	VITE_WAMP_WT_CERT_URL=http://localhost:8083/wt-cert-hash \
	npm run dev

type-check:
	npm run type-check

# Type-check, build, then AppImage/deb (or dmg/exe) in release/.
build:
	npm run package

clean:
	rm -rf dist/ release/ node_modules/

.PHONY: setup run run-local type-check build clean
