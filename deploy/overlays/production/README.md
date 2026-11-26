# Production overlay

This overlay starts the API in `TSP_ENV=production` with:

```text
demo mode off
JT808 command mode
PII/export encryption required
Redis TLS URL placeholder
CORS origin placeholder
```

Validate only:

```bash
kubectl kustomize deploy/overlays/production > /tmp/tsp-production.yaml
```

Do not apply until the `tsp-secret` Secret contains all keys referenced by the
base manifests **plus**:

```text
pii-key
export-key
```

Example (merge into the existing `tsp-secret`, do not replace JWT/RustFS keys):

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: tsp-secret
  namespace: xingtu-tsp
stringData:
  pii-key: "REPLACE_WITH_32_BYTE_HEX_OR_BASE64"
  export-key: "REPLACE_WITH_32_BYTE_BASE64_OR_HEX"
```

Replace before apply:

```text
TSP_REDIS_URL       managed Redis with TLS (rediss://)
TSP_CORS_ORIGINS    customer HTTPS origin
TSP_POSTGIS_ENABLED true only when KGIS/PostGIS is available
```

The overlay deliberately uses `.example.invalid` placeholders; it must not be
applied as-is.

## Current limits

- This is a manifest-level production overlay, not a production certification.
- KGIS, real terminal/platform/video interop, official conformance, multi-AZ
  database and security assessment remain external P0 items.
- `kubectl kustomize` validation is not a cluster deployment test.
