data "ovh_me" "account" {}

resource "ovh_vps" "cache" {
  display_name         = var.name
  image_id             = var.image_id
  ovh_subsidiary       = data.ovh_me.account.ovh_subsidiary
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
}

data "ovh_vps" "cache" {
  service_name = ovh_vps.cache.name
}

locals {
  vps_addresses = [for address in data.ovh_vps.cache.ips : split("/", address)[0]]
  vps_ipv4      = one([for address in local.vps_addresses : address if !strcontains(address, ":")])
}

resource "cloudflare_r2_bucket" "cache" {
  account_id    = var.cloudflare_account_id
  name          = var.r2_bucket
  location      = "enam"
  storage_class = "Standard"
}

resource "cloudflare_dns_record" "cache" {
  zone_id = var.cloudflare_zone_id
  name    = var.domain
  content = local.vps_ipv4
  type    = "A"
  ttl     = 300
  proxied = false
  comment = "mise-cache origin managed by Terraform"
}
