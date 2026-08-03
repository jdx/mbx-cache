output "server_ipv4" {
  value = hcloud_server.cache.ipv4_address
}

output "server_ipv6" {
  value = hcloud_server.cache.ipv6_address
}

output "cache_url" {
  value = "https://${var.domain}"
}

output "object_storage_endpoint" {
  value = "https://${var.object_storage_location}.your-objectstorage.com"
}

output "object_storage_bucket" {
  value = minio_s3_bucket.cache.bucket
}
