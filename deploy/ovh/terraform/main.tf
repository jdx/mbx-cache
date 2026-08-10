locals {
  create_vps = var.existing_server_ipv4 == null
}

data "ovh_me" "account" {
  count = local.create_vps ? 1 : 0
}

resource "ovh_vps" "cache" {
  count = local.create_vps ? 1 : 0

  display_name         = var.name
  image_id             = var.image_id
  ovh_subsidiary       = data.ovh_me.account[0].ovh_subsidiary
  public_ssh_key       = var.public_ssh_key
  do_not_send_password = true

  plan = [{
    duration     = var.plan_duration
    plan_code    = var.plan_code
    pricing_mode = var.pricing_mode
    configuration = [
      {
        label = "vps_datacenter"
        value = var.datacenter
      },
      {
        label = "vps_os"
        value = var.operating_system
      }
    ]
  }]

  lifecycle {
    prevent_destroy = true

    precondition {
      condition     = var.plan_code != null
      error_message = "plan_code is required when existing_server_ipv4 is unset."
    }
    precondition {
      condition     = var.image_id != null
      error_message = "image_id is required when existing_server_ipv4 is unset."
    }
    precondition {
      condition     = var.public_ssh_key != null
      error_message = "public_ssh_key is required when existing_server_ipv4 is unset."
    }
  }
}

data "ovh_vps" "cache" {
  count        = local.create_vps ? 1 : 0
  service_name = ovh_vps.cache[0].name
}

locals {
  created_vps_addresses = local.create_vps ? [for address in data.ovh_vps.cache[0].ips : split("/", address)[0]] : []
  vps_ipv4 = local.create_vps ? one([
    for address in local.created_vps_addresses : address if !strcontains(address, ":")
  ]) : var.existing_server_ipv4
}
