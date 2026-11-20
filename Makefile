SHELL := /bin/bash
CARGO ?= cargo
SSH_HOST ?= root@192.168.88.38
LOG_DIR ?= output
BIN := target/release/melodias-studio

.PHONY: help run generate build check fmt test deploy status log clean

help:
	@printf '%s\n' \
	  'melodias-studio' \
	  '' \
	  '  make run                             TUI' \
	  '  make generate URL=... VIBE="..."     pipeline sin TUI, con log en output/' \
	  '  make log                             ultimas lineas del log mas reciente' \
	  '  make check                           fmt + build + clippy -D warnings + tests' \
	  '  make status                          estado de llm, sd y audio en la VM' \
	  '  make deploy                          copia deploy/ a la VM' \
	  '  make clean                           cargo clean'

run:
	$(CARGO) run

generate:
	@test -n "$(URL)" || { echo 'falta URL, por ejemplo: make generate URL="https://www.youtube.com/watch?v=..." VIBE="epic 8-bit orchestral finale"'; exit 2; }
	@test -n "$(VIBE)" || { echo 'falta VIBE="..."'; exit 2; }
	@mkdir -p "$(LOG_DIR)"
	@$(CARGO) build --release
	@log="$(LOG_DIR)/run-$$(date +%Y%m%d-%H%M%S).log"; \
	  MELODIAS_LOG="$$log" $(BIN) "$(URL)" "$(VIBE)"

build:
	$(CARGO) build

check:
	$(CARGO) fmt --check
	$(CARGO) build
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) test

fmt:
	$(CARGO) fmt

test:
	$(CARGO) test

deploy:
	ssh $(SSH_HOST) 'mkdir -p /home/ubuntu/bin'
	scp deploy/serve-*.sh $(SSH_HOST):/home/ubuntu/bin/
	scp deploy/audio-server.json $(SSH_HOST):/home/ubuntu/
	ssh $(SSH_HOST) 'chmod +x /home/ubuntu/bin/serve-*.sh'

status:
	@ssh $(SSH_HOST) 'for s in llm sd audio; do printf "%-6s " "$$s"; /home/ubuntu/bin/serve-$$s.sh status 2>&1 | tail -1; done'

log:
	@latest=$$(ls -t $(LOG_DIR)/*.log 2>/dev/null | head -1); \
	  if [ -z "$$latest" ]; then echo "no hay logs en $(LOG_DIR)"; else echo "== $$latest =="; tail -n 40 "$$latest"; fi

clean:
	$(CARGO) clean
