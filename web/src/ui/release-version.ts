const LATEST_RELEASE_URL =
  "https://api.github.com/repos/driftcell/RMCL/releases/latest";
const CACHE_KEY = "rmcl:latest-release-version";
const CACHE_TTL_MS = 60 * 60 * 1000;
const REQUEST_TIMEOUT_MS = 8_000;

interface GitHubRelease {
  tag_name?: unknown;
}

interface CachedRelease {
  version: string;
  fetchedAt: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function normalizeVersion(tagName: unknown): string | null {
  if (typeof tagName !== "string") return null;
  const tag = tagName.trim();
  const version = tag.startsWith("v") || tag.startsWith("V") ? tag.slice(1) : tag;
  return /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)
    ? version
    : null;
}

function readCachedRelease(): CachedRelease | null {
  try {
    const raw = sessionStorage.getItem(CACHE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed)) return null;
    const { version, fetchedAt } = parsed;
    if (typeof version !== "string" || typeof fetchedAt !== "number") {
      return null;
    }
    if (!normalizeVersion(version)) return null;
    return { version, fetchedAt };
  } catch {
    return null;
  }
}

function writeCachedRelease(version: string): void {
  try {
    sessionStorage.setItem(
      CACHE_KEY,
      JSON.stringify({ version, fetchedAt: Date.now() }),
    );
  } catch {
    // Storage can be unavailable in private or restricted browsing contexts.
  }
}

function updateStructuredData(version: string): void {
  const script = document.querySelector<HTMLScriptElement>(
    'script[type="application/ld+json"]',
  );
  if (!script?.textContent) return;

  try {
    const data: unknown = JSON.parse(script.textContent);
    if (!isRecord(data) || !Array.isArray(data["@graph"])) return;

    const app = data["@graph"].find(
      (node): node is Record<string, unknown> =>
        isRecord(node) && node["@type"] === "SoftwareApplication",
    );
    if (!app) return;

    app.softwareVersion = version;
    script.textContent = JSON.stringify(data);
  } catch {
    // Keep the static structured data untouched if it cannot be parsed.
  }
}

function applyVersion(version: string): void {
  for (const element of document.querySelectorAll<HTMLElement>(
    "[data-release-version]",
  )) {
    element.textContent = `v${version}`;
    element.hidden = false;
  }
  updateStructuredData(version);
}

async function fetchLatestRelease(): Promise<string> {
  const controller = new AbortController();
  const timeout = window.setTimeout(
    () => controller.abort(),
    REQUEST_TIMEOUT_MS,
  );

  try {
    const response = await fetch(LATEST_RELEASE_URL, {
      headers: { Accept: "application/vnd.github+json" },
      signal: controller.signal,
    });
    if (!response.ok) {
      throw new Error(`GitHub API responded with ${response.status}`);
    }

    const release = (await response.json()) as GitHubRelease;
    const version = normalizeVersion(release.tag_name);
    if (!version) {
      throw new Error("GitHub release tag is not a supported version");
    }
    return version;
  } finally {
    window.clearTimeout(timeout);
  }
}

/**
 * Shows the latest published RMCL release version on the static site.
 *
 * The page stays static and asks the public GitHub REST API at runtime so a
 * launcher release does not require a web redeploy. A successful lookup is
 * cached for the current browser tab to stay well below the unauthenticated
 * API rate limit; a stale cached version is kept when a refresh fails.
 */
export function syncReleaseVersion(): void {
  const cached = readCachedRelease();
  if (cached) {
    applyVersion(cached.version);
    if (Date.now() - cached.fetchedAt < CACHE_TTL_MS) {
      return;
    }
  }

  void fetchLatestRelease()
    .then((version) => {
      applyVersion(version);
      writeCachedRelease(version);
    })
    .catch((error: unknown) => {
      if (cached) return;
      console.warn(
        "[release-version] Unable to read the latest RMCL release.",
        error,
      );
    });
}
