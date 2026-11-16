.PHONY: release release-publish release-android

release:
	bash scripts/release.sh "$(VERSION)" --android "$(ANDROID_DIR)" --report "$(CHECK_REPORT)"
release-publish:
	bash scripts/release.sh "$(VERSION)" --publish --android "$(ANDROID_DIR)" --report "$(CHECK_REPORT)"

release-android:
	bash scripts/release_android.sh "$(VERSION)" "$(OUT_DIR)"
