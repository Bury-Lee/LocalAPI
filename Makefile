SHELL := cmd.exe
.SHELLFLAGS := /C

CARGO   ?= cargo
BIN     := LocalAPI.exe
DIST    := dist
RELEASE := target\release

.PHONY: all build dist drivers clean run

all: dist

build:
	$(CARGO) build --release

drivers:
	@if exist "$(DIST)" rmdir /S /Q "$(DIST)\cjs"
	if not exist "$(DIST)" mkdir "$(DIST)"
	xcopy /E /I /Y "cjs" "$(DIST)\cjs"

dist: build drivers
	copy /Y "$(RELEASE)\$(BIN)" "$(DIST)\$(BIN)"
	@if exist "config.json" copy /Y "config.json" "$(DIST)\config.json"
	@echo.
	@echo dist ready: $(DIST)\$(BIN) + $(DIST)\cjs\*-driver.cjs

run: dist
	cd "$(DIST)" && "$(BIN)"

clean:
	$(CARGO) clean
	@if exist "$(DIST)" rmdir /S /Q "$(DIST)"
