terraform {
  required_version = ">= 1.8.0"

  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.66"
    }
    minio = {
      source  = "aminueza/minio"
      version = "~> 3.33"
    }
  }
}

provider "cloudflare" {}
provider "hcloud" {}

provider "minio" {
  minio_server   = "${var.object_storage_location}.your-objectstorage.com"
  minio_user     = var.object_storage_access_key
  minio_password = var.object_storage_secret_key
  minio_region   = var.object_storage_location
  minio_ssl      = true
}
