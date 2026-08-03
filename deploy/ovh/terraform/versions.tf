terraform {
  required_version = ">= 1.8.0"

  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
    ovh = {
      source  = "ovh/ovh"
      version = "~> 2.17"
    }
  }
}

provider "cloudflare" {}
provider "ovh" {}
