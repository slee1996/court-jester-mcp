from __future__ import annotations

import sys
from pathlib import Path


def rewrite(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    if old not in text:
        raise SystemExit(f"Could not find target snippet in {path}")
    path.write_text(text.replace(old, new, 1))


def rewrite_any(paths: list[Path], old: str, new: str) -> None:
    for path in paths:
        if not path.exists():
            continue
        text = path.read_text()
        if old not in text:
            continue
        path.write_text(text.replace(old, new, 1))
        return
    searched = ", ".join(str(path) for path in paths)
    raise SystemExit(f"Could not find target snippet in any of: {searched}")


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: seed_express_clone_bug.py <workspace> <bug_id>")

    workspace = Path(sys.argv[1])
    bug_id = sys.argv[2]

    index_path = workspace / "index.ts"
    http_path = workspace / "lib" / "http.ts"
    query_path = workspace / "lib" / "query.ts"
    router_path = workspace / "lib" / "router.ts"

    if bug_id == "router_missing_method":
        rewrite_any(
            [router_path, index_path],
            'const method = (request.method || "").toUpperCase();',
            'const method = (request.method || "GET").toUpperCase();',
        )
        return 0

    if bug_id == "app_use_mounting":
        rewrite_any(
            [router_path, index_path],
            "  maybeApp.parent = parent;\n  maybeApp.__eventEmitter?.emit(\"mount\", parent);\n",
            "",
        )
        return 0

    if bug_id == "app_param_cache":
        rewrite_any(
            [router_path, index_path],
            "                if (paramState.get(key) === value) {\n                  paramIndex += 1;\n                  continue;\n                }\n",
            "",
        )
        return 0

    if bug_id == "req_baseurl":
        rewrite_any(
            [router_path, index_path],
            '                request.baseUrl = `${previousBaseUrl}${match.matchedPath}`;\n',
            "                request.baseUrl = previousBaseUrl;\n",
        )
        return 0

    if bug_id == "req_query_extended":
        rewrite(
            query_path,
            '  if (setting === "extended") {\n    return parseExtended(input);\n  }\n',
            '  if (setting === "extended") {\n    return parseSimple(input);\n  }\n',
        )
        return 0

    if bug_id == "res_format_qvalues":
        rewrite_any(
            [http_path, index_path],
            "    if (!best || candidateQ > best.q || (candidateQ === best.q && candidateOrder < best.order)) {\n      best = { option, q: candidateQ, order: candidateOrder };\n    }\n",
            "    if (!best || candidateOrder < best.order) {\n      best = { option, q: candidateQ, order: candidateOrder };\n    }\n",
        )
        return 0

    if bug_id == "res_redirect_encoding":
        rewrite_any(
            [http_path, index_path],
            "      const encodedUrl = encodeRedirectUrl(rawUrl);\n",
            "      const encodedUrl = rawUrl;\n",
        )
        return 0

    if bug_id == "route_all_ignored":
        rewrite_any(
            [router_path, index_path],
            '    const entries = route.entries.filter((entry) => entry.method === "ALL" || entry.method === method);\n',
            '    const entries = route.entries.filter((entry) => entry.method === method);\n',
        )
        return 0

    if bug_id == "res_send_numbers_as_text":
        rewrite_any(
            [http_path, index_path],
            "      if (shouldUseJsonSend(body)) {\n        return response.json?.(body);\n      }\n",
            "      if (body !== null && typeof body === \"object\" && !ArrayBuffer.isView(body)) {\n        return response.json?.(body);\n      }\n",
        )
        return 0

    if bug_id == "res_json_escape_disabled":
        rewrite_any(
            [http_path, index_path],
            '      if (request.app?.__settings.get("json escape") === true && payload) {\n        payload = escapeJsonString(payload);\n      }\n',
            "",
        )
        return 0

    if bug_id == "app_router_params_restore":
        rewrite_any(
            [router_path, index_path],
            "                      if ((callback as RouterLike).__isExpressRouter || (callback as ApplicationLike).__isExpressApp) {\n                        const nestedPreviousParams = request.params;\n                        request.params = previousParams;\n                        (callback as RouterLike)(request, response, (arg?: unknown) => {\n                          request.params = nestedPreviousParams;\n                          if (arg === \"route\") {\n                            finishRoute();\n                            return;\n                          }\n                          currentError = arg;\n                          runRoute();\n                        });\n                      } else {\n",
            "                      if ((callback as RouterLike).__isExpressRouter || (callback as ApplicationLike).__isExpressApp) {\n                        (callback as RouterLike)(request, response, (arg?: unknown) => {\n                          if (arg === \"route\") {\n                            finishRoute();\n                            return;\n                          }\n                          currentError = arg;\n                          runRoute();\n                        });\n                      } else {\n",
        )
        return 0

    if bug_id == "app_routes_error_skip_handlers":
        rewrite_any(
            [router_path, index_path],
            "                    if (currentError !== undefined) {\n                      callback(currentError, request, response, (arg?: unknown) => {\n                        if (arg === \"route\") {\n                          finishRoute();\n                          return;\n                        }\n                        currentError = arg;\n                        runRoute();\n                      });\n                    } else {\n",
            "                    if (currentError !== undefined) {\n                      callback(request, response, (arg?: unknown) => {\n                        if (arg === \"route\") {\n                          finishRoute();\n                          return;\n                        }\n                        currentError = arg;\n                        runRoute();\n                      });\n                    } else {\n",
        )
        return 0

    if bug_id == "express_json_passthrough":
        rewrite(
            index_path,
            "      const parsed = JSON.parse(body);\n",
            "      const parsed = body;\n",
        )
        return 0

    if bug_id == "express_urlencoded_simple_only":
        rewrite(
            index_path,
            '  const parserSetting = options?.extended ? "extended" : "simple";\n',
            '  const parserSetting = "simple";\n',
        )
        return 0

    if bug_id == "req_get_case_sensitive":
        rewrite_any(
            [http_path, index_path],
            "function requestHeader(request: RequestLike, name: string): string | undefined {\n  const target = normalizeHeaderName(name);\n  const aliases =\n    target === \"referer\" || target === \"referrer\"\n      ? new Set([\"referer\", \"referrer\"])\n      : new Set([target]);\n  for (const [headerName, headerValue] of Object.entries(request.headers || {})) {\n    if (aliases.has(normalizeHeaderName(headerName))) {\n      return headerValue;\n    }\n  }\n  return undefined;\n}\n",
            "function requestHeader(request: RequestLike, name: string): string | undefined {\n  return request.headers?.[name];\n}\n",
        )
        return 0

    if bug_id == "res_sendstatus_numeric_body":
        rewrite_any(
            [http_path, index_path],
            '      const text = STATUS_TEXT[code] || String(code);\n',
            '      const text = String(code);\n',
        )
        return 0

    if bug_id == "express_text_uppercase":
        rewrite(
            index_path,
            "    req.body = requestBodyText(req);\n",
            "    req.body = requestBodyText(req).toUpperCase();\n",
        )
        return 0

    if bug_id == "express_raw_string_body":
        rewrite(
            index_path,
            '    req.body = Buffer.from(requestBodyText(req), "utf8");\n',
            "    req.body = requestBodyText(req);\n",
        )
        return 0

    if bug_id == "req_protocol_ignores_proxy":
        rewrite_any(
            [http_path, index_path],
            '    request.protocol = trustProxy && forwardedProtocol ? forwardedProtocol : encrypted ? "https" : "http";\n',
            '    request.protocol = encrypted ? "https" : "http";\n',
        )
        return 0

    if bug_id == "res_location_unencoded":
        rewrite_any(
            [http_path, index_path],
            '      response.setHeader?.("Location", encodeRedirectUrl(target));\n',
            '      response.setHeader?.("Location", target);\n',
        )
        return 0

    if bug_id == "res_links_overwrite":
        rewrite_any(
            [http_path, index_path],
            '      response.setHeader?.("Link", appendHeaderValue(response.getHeader?.("link"), serialized));\n',
            '      response.setHeader?.("Link", serialized);\n',
        )
        return 0

    if bug_id == "res_vary_reset":
        rewrite_any(
            [http_path, index_path],
            '      const existing = response.getHeader?.("vary");\n',
            '      const existing = undefined;\n',
        )
        return 0

    raise SystemExit(f"Unknown bug id: {bug_id}")


if __name__ == "__main__":
    raise SystemExit(main())
