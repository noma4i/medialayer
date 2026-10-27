.PHONY: release release-publish

release:
	bash scripts/release.sh "$(VERSION)"
release-publish:
	bash scripts/release.sh "$(VERSION)" --publish
