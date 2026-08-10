variable "name" {
  description = "Display name for the VPS."
  type        = string
  default     = "mise-cache"
}

variable "domain" {
  description = "Public cache hostname, for example cache.mise.jdx.dev."
  type        = string
}

variable "cloudflare_account_id" {
  description = "Cloudflare account ID that owns the R2 bucket."
  type        = string
}

variable "r2_bucket" {
  description = "Existing R2 bucket name for cache blobs."
  type        = string
}

variable "existing_server_ipv4" {
  description = "Existing VPS IPv4 address to adopt instead of ordering a new OVH VPS."
  type        = string
  default     = null
  nullable    = true

  validation {
    condition = (
      var.existing_server_ipv4 == null ||
      (!strcontains(var.existing_server_ipv4, ":") &&
      can(cidrhost("${var.existing_server_ipv4}/32", 0)))
    )
    error_message = "existing_server_ipv4 must be a valid IPv4 address."
  }
}

variable "plan_code" {
  description = "Currently available OVH US VPS plan code from the OVH order catalog."
  type        = string
  default     = null
  nullable    = true
}

variable "plan_duration" {
  description = "OVH billing duration."
  type        = string
  default     = "P1M"
}

variable "pricing_mode" {
  description = "OVH pricing mode returned by the order catalog."
  type        = string
  default     = "default"
}

variable "datacenter" {
  description = "OVH VPS datacenter. US-EAST-VA places the cache in Vint Hill, Virginia."
  type        = string
  default     = "US-EAST-VA"
}

variable "operating_system" {
  description = "Operating-system label returned by the OVH order catalog."
  type        = string
  default     = "Ubuntu 24.04"
}

variable "image_id" {
  description = "OVH Ubuntu image ID used to install the supplied SSH key."
  type        = string
  default     = null
  nullable    = true
}

variable "public_ssh_key" {
  description = "Public SSH key installed on the VPS."
  type        = string
  default     = null
  nullable    = true
}
