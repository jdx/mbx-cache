variable "name" {
  description = "Name used for the server and related resources."
  type        = string
  default     = "mise-cache"
}

variable "domain" {
  description = "Public cache hostname, for example cache.mise.jdx.dev."
  type        = string
}

variable "cloudflare_zone_id" {
  description = "Cloudflare zone ID containing the cache hostname."
  type        = string
  sensitive   = true
}

variable "server_type" {
  description = "Hetzner Cloud server type. CX23 provides 2 shared vCPUs and 4 GB RAM."
  type        = string
  default     = "cx23"
}

variable "location" {
  description = "Hetzner location. Keep this in the same network zone as Object Storage."
  type        = string
  default     = "nbg1"
}

variable "ssh_key_name" {
  description = "Name of an SSH key already registered in the Hetzner Cloud project."
  type        = string
}

variable "ssh_source_cidrs" {
  description = "CIDRs allowed to SSH to the server. An empty list disables public SSH."
  type        = list(string)
  default     = []
}

variable "enable_backups" {
  description = "Enable Hetzner's seven-slot server backups for the local PostgreSQL data."
  type        = bool
  default     = true
}

variable "object_storage_location" {
  description = "Hetzner Object Storage location."
  type        = string
  default     = "nbg1"
}

variable "object_storage_bucket" {
  description = "Globally unique bucket name for cache blobs."
  type        = string
}

variable "object_storage_access_key" {
  description = "Hetzner Object Storage access key. Supply with TF_VAR_object_storage_access_key."
  type        = string
  sensitive   = true
}

variable "object_storage_secret_key" {
  description = "Hetzner Object Storage secret key. Supply with TF_VAR_object_storage_secret_key."
  type        = string
  sensitive   = true
}
