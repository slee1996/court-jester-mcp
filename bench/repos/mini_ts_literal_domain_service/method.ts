export type HttpMethod = "GET" | "PATCH";

const METHOD_RULES: Partial<Record<HttpMethod, { allowsBody: boolean }>> = {
  GET: { allowsBody: false },
};

export function methodAllowsBody(method: HttpMethod): boolean {
  return METHOD_RULES[method]!.allowsBody;
}
