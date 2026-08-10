output "server_ipv4" {
  value = local.vps_ipv4
}

output "vps_service_name" {
  value = local.create_vps ? ovh_vps.cache[0].name : null
}

output "cache_url" {
  value = "https://${var.domain}"
}

output "r2_endpoint" {
  value = "https://${var.cloudflare_account_id}.r2.cloudflarestorage.com"
}

output "r2_bucket" {
  value = var.r2_bucket
}
