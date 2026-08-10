terraform {
  required_version = ">= 1.8.0"

  required_providers {
    ovh = {
      source  = "ovh/ovh"
      version = "~> 2.17"
    }
  }
}

provider "ovh" {
  ignore_init_error = !local.create_vps
}
