SHELL := cmd.exe
.SHELLFLAGS := /C

CARGO   ?= cargo
BIN     := LocalAPI.exe
DIST    := dist
RELEASE := target\release
PKG     := $(DIST)\release
PKGNAME := LocalAPI
STAGE   := $(PKG)\$(PKGNAME)
ARCHIVE := $(PKG)\$(PKGNAME).7z
SEVENZIP ?= 7z

.PHONY: all build dist drivers release clean run

all: release

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

# 运行文件 + 默认配置打成一个 7z，只把压缩包留在 dist\release
release: dist
	@if exist "$(STAGE)" rmdir /S /Q "$(STAGE)"
	if not exist "$(STAGE)" mkdir "$(STAGE)"
	copy /Y "$(DIST)\$(BIN)" "$(STAGE)\$(BIN)"
	xcopy /E /I /Y "$(DIST)\cjs" "$(STAGE)\cjs"
	"$(DIST)\$(BIN)" init --path "$(STAGE)\config.json" --force
	@if exist "README.md" copy /Y "README.md" "$(STAGE)\README.md"
	@if exist "$(ARCHIVE)" del /Q "$(ARCHIVE)"
	cd /D "$(PKG)" && $(SEVENZIP) a -t7z -mx=9 "$(PKGNAME).7z" "$(PKGNAME)" >NUL && rmdir /S /Q "$(PKGNAME)"
	@echo.
	@echo release ready: $(ARCHIVE)

run: dist
	cd "$(DIST)" && "$(BIN)"

clean:
	$(CARGO) clean
	@if exist "$(DIST)" rmdir /S /Q "$(DIST)"
