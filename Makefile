.PHONY: release release-publish release-android

release:
	bash scripts/release.sh "$(VERSION)" --android "$(ANDROID_DIR)"
release-publish:
	bash scripts/release.sh "$(VERSION)" --publish --android "$(ANDROID_DIR)"

release-android:
	bash scripts/release_android.sh "$(VERSION)" "$(OUT_DIR)"
