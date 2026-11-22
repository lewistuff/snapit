APP      := build/snapit.app
BIN      := target/release/snapit
export MACOSX_DEPLOYMENT_TARGET := 14.0

.PHONY: test build app run measure clean

test:
	cargo test

build:
	cargo build --release

app: build
	rm -rf $(APP)
	mkdir -p $(APP)/Contents/MacOS
	cp build/Info.plist $(APP)/Contents/Info.plist
	mkdir -p $(APP)/Contents/Resources
	cp $(BIN) $(APP)/Contents/MacOS/snapit
	cp assets/AppIcon.icns $(APP)/Contents/Resources/AppIcon.icns
	scripts/sign.sh $(APP)

run: app
	-pkill -x snapit
	open $(APP)

measure: app
	scripts/measure.sh $(APP)

clean:
	cargo clean
	rm -rf $(APP)
