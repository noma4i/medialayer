.PHONY: release release-publish release-artifacts release-android

release:
	bash scripts/release.sh "$(VERSION)" --android "$(ANDROID_DIR)"
release-publish:
	bash scripts/release.sh "$(VERSION)" --publish --android "$(ANDROID_DIR)"
release-artifacts:
	bash scripts/release.sh "$(VERSION)" --artifacts "$(or $(MODE),dry)" $(if $(STAGE),--stage "$(STAGE)") $(if $(REMOTE),--remote "$(REMOTE)")

release-android:
	bash scripts/release_android.sh "$(VERSION)" "$(OUT_DIR)"
