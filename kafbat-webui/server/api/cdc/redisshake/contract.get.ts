import { orchestratorFetch } from "~/server/utils/orchestratorFetch"

export default defineEventHandler((event) => orchestratorFetch(event, "/cdc/redisshake/contract"))
