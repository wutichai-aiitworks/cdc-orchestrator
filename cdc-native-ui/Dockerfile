FROM node:22-bookworm AS builder
WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci

COPY . .
RUN npm run build

FROM node:22-bookworm-slim AS runtime
WORKDIR /app
ENV NODE_ENV=production
ENV HOST=0.0.0.0
ENV PORT=3000

COPY --from=builder /app/.output ./.output
COPY map-env.mjs start-ui.mjs /app/

# Shown on the login page. Last on purpose: a per-commit value must not bust the build cache.
ARG APP_VERSION=dev
ENV APP_VERSION=${APP_VERSION}

USER node
# 3000 = UI (AUTH_MODE as configured); ADMIN_PORT (e.g. 3001) = break-glass local login.
EXPOSE 3000 3001
# Checks the UI port and, when ADMIN_PORT is set, the admin port too.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["node", "-e", "const e=process.env;Promise.all([e.PORT||'3000',e.ADMIN_PORT].filter(Boolean).map((p)=>fetch('http://127.0.0.1:'+p+'/').then((r)=>{if(!r.ok)throw r.status}))).catch(()=>process.exit(1))"]
ENTRYPOINT ["node", "/app/start-ui.mjs"]
CMD ["./.output/server/index.mjs"]
